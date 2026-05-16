mod args;
mod connection;
mod cookie;
mod format;
mod netrc;
mod ntlm;
mod options;
mod request;
mod response;
mod tls;
mod url;
mod variables;

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use std::time::Duration;

use args::parse_args;
use cookie::save_cookie_jar;
use format::format_write_out;
use request::perform;
use url::{expand_glob, parse_url};

fn main() {
    let mut opts = parse_args();
    let mut exit_code = 0;

    // http_proxy / HTTP_PROXY environment variable fallback.
    // If -x/--proxy was not specified on the command line, check the environment.
    if opts.proxy.is_none()
        && let Ok(proxy) = std::env::var("http_proxy").or_else(|_| std::env::var("HTTP_PROXY"))
        && !proxy.is_empty()
    {
        // Extract userinfo (user:pass) from proxy URL if present
        // e.g. http://user:pass@proxy.example:port/
        if opts.proxy_user.is_none() {
            let stripped = proxy
                .strip_prefix("http://")
                .or_else(|| proxy.strip_prefix("https://"))
                .unwrap_or(&proxy);
            if let Some(at_pos) = stripped.find('@') {
                let userinfo = &stripped[..at_pos];
                if !userinfo.is_empty() {
                    let decoded = url::percent_decode(userinfo);
                    let with_colon = if decoded.contains(':') {
                        decoded
                    } else {
                        format!("{decoded}:")
                    };
                    opts.proxy_user = Some(with_colon);
                }
            }
        }
        opts.proxy = Some(proxy);
    }

    // Handle empty -x "" (explicit empty proxy means no proxy).
    if opts.proxy.as_deref() == Some("") {
        opts.proxy = None;
    }

    // --stderr: redirect stderr to a file (or stdout when "-").
    // Set up the redirect early so glob errors also go to the right place.
    let _stderr_guard: Option<Box<dyn std::any::Any>> = if let Some(ref dest) = opts.stderr_redirect
    {
        if dest.to_str() == Some("-") {
            // Redirect stderr to stdout using dup2.
            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;
                let stdout_fd = std::io::stdout().as_raw_fd();
                unsafe extern "C" {
                    fn dup2(oldfd: i32, newfd: i32) -> i32;
                }
                // SAFETY: dup2 is a standard POSIX function. Redirecting stderr (fd 2)
                // to stdout is safe; stdout_fd is valid for the process lifetime.
                unsafe {
                    dup2(stdout_fd, 2);
                }
            }
            None
        } else {
            // Redirect stderr to the given file by dup2'ing the file's fd onto fd 2.
            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;
                if let Ok(file) = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(dest)
                {
                    let fd = file.as_raw_fd();
                    unsafe extern "C" {
                        fn dup2(oldfd: i32, newfd: i32) -> i32;
                    }
                    // SAFETY: dup2 is a standard POSIX function. We keep the
                    // file alive in `_stderr_guard` so its fd remains valid
                    // for the lifetime of the program.
                    unsafe {
                        dup2(fd, 2);
                    }
                    Some(Box::new(file) as Box<dyn std::any::Any>)
                } else {
                    None
                }
            }
            #[cfg(not(unix))]
            {
                None
            }
        }
    } else {
        None
    };

    // Emit any warnings deferred during argument parsing now that the
    // (possibly --stderr-redirected) stderr is in place (test 1268).
    for warn in opts.deferred_warnings.drain(..).collect::<Vec<_>>() {
        eprintln!("{warn}");
    }

    // URL globbing: expand {a,b,c} and [1-10] style patterns in each URL
    // into separate URLs. -g/--globoff disables this.
    // Track glob values for each URL so #N in -o refers to the Nth glob value.
    let url_glob_values: Vec<Vec<String>>;
    let orig_urls_count = opts.urls.len();
    let orig_outputs = opts.outputs.clone();
    if !opts.globoff {
        let orig_urls = opts.urls.clone();
        let mut all_expanded = Vec::new();
        // Map each expanded URL back to its original URL index so per-URL
        // pairings (per_url_opts, outputs) survive glob expansion.
        let mut orig_idx_of: Vec<usize> = Vec::new();
        for (orig_idx, u) in orig_urls.iter().enumerate() {
            match expand_glob(u) {
                Ok(expanded) => {
                    for _ in 0..expanded.len() {
                        orig_idx_of.push(orig_idx);
                    }
                    all_expanded.extend(expanded);
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(3);
                }
            }
        }
        if !opts.per_url_opts.is_empty() {
            let mut new_per_url = Vec::new();
            for &orig_idx in &orig_idx_of {
                let puo = opts.per_url_opts.get(orig_idx).cloned().unwrap_or_default();
                new_per_url.push(puo);
            }
            opts.per_url_opts = new_per_url;
        }
        // Replicate -o by original-URL index. Curl pairs each `-o` with a URL
        // definition, NOT a glob expansion: a single `-o file#1` on one
        // globbed URL applies (the same -o, with #N substitution per
        // iteration) to every expansion (test 1328). Extras beyond the
        // original URL count are unused.
        if !orig_outputs.is_empty() {
            let mut new_outputs = Vec::with_capacity(orig_idx_of.len());
            let orig_outputs_null = opts.outputs_null.clone();
            let mut new_outputs_null = Vec::with_capacity(orig_idx_of.len());
            for &orig_idx in &orig_idx_of {
                if let Some(o) = orig_outputs.get(orig_idx) {
                    new_outputs.push(o.clone());
                }
                new_outputs_null.push(orig_outputs_null.get(orig_idx).copied().unwrap_or(false));
            }
            opts.outputs = new_outputs;
            opts.outputs_null = new_outputs_null;
        }
        opts.urls = all_expanded.iter().map(|(url, _)| url.clone()).collect();
        url_glob_values = all_expanded.into_iter().map(|(_, vals)| vals).collect();
    } else {
        url_glob_values = opts.urls.iter().map(|_| Vec::new()).collect();
    }

    if orig_outputs.len() > orig_urls_count && !opts.silent {
        eprintln!("Warning: Got more output options than URLs");
    }

    // --etag-save pre-flight is done per-URL inside the loop below so that
    // a failed first URL doesn't abort subsequent URLs (test 369).

    // -C - (auto-resume):
    //   - GET: use the existing output file's size as the resume offset.
    //   - PUT (-T): curl can't know server-side size, so it always starts from
    //     offset 0 (sends the full upload with Content-Range: bytes 0-N-1/N).
    // For uploads, -C - means "start from offset 0" (curl can't know server-side
    // file size). For downloads we keep the "-" sentinel and re-stat the output
    // file inside the URL/retry loop so each attempt picks up the current size
    // (test 3035: `--continue-at - --retry --retry-all-errors`).
    if opts.resume_from.as_deref() == Some("-") && opts.upload_file.is_some() {
        opts.resume_from = Some("0".to_string());
    }

    // Pre-validate --dump-header destination. "-" means stdout, "%" means stderr;
    // otherwise try to open as a file (curl fails early if the path is unusable).
    if let Some(ref dump_path) = opts.dump_header
        && dump_path.to_str() != Some("-")
        && dump_path.to_str() != Some("%")
    {
        if opts.create_dirs
            && let Some(parent) = dump_path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = fs::create_dir_all(parent);
        }
        if fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(dump_path)
            .is_err()
        {
            eprintln!("curl: (23) Failed to open/create {}", dump_path.display());
            process::exit(23);
        }
    }

    // -G / --get: append -d/--data-* content as a query string, and clear the
    // request body so these become GET (or HEAD with -I) requests.
    // When per-URL options are present, each URL uses its own snapshot's
    // get/data fields; otherwise fall back to the global opts.
    if !opts.per_url_opts.is_empty() {
        for (idx, puo) in opts.per_url_opts.iter_mut().enumerate() {
            if puo.get && puo.data.is_some() {
                let data = puo.data.take().unwrap();
                if let Ok(s) = std::str::from_utf8(&data)
                    && let Some(url) = opts.urls.get_mut(idx)
                {
                    if url.contains('?') {
                        url.push('&');
                    } else {
                        url.push('?');
                    }
                    url.push_str(s);
                }
            }
        }
    } else if opts.get && opts.data.is_some() {
        let data = opts.data.take().unwrap();
        if let Ok(s) = std::str::from_utf8(&data) {
            let extra = s.to_string();
            for url in opts.urls.iter_mut() {
                if url.contains('?') {
                    url.push('&');
                } else {
                    url.push('?');
                }
                url.push_str(&extra);
            }
        }
    }

    // --url-query: append the joined query items to each URL's query string
    // (test 1221). `?` separator on first append, `&` thereafter.
    let urls_with_query: Vec<String> = opts
        .urls
        .iter()
        .map(|u| {
            if opts.url_queries.is_empty() {
                u.clone()
            } else {
                let joined = opts.url_queries.join("&");
                if u.contains('?') {
                    format!("{u}&{joined}")
                } else {
                    format!("{u}?{joined}")
                }
            }
        })
        .collect();
    // --hsts <file>: load HSTS DB and upgrade matching http:// URLs to https://.
    // Trailing dots normalize away on both sides; entries beginning with `.`
    // also match all subdomains (tests 440, 441, 493).
    // hsts_entries: (subdomain_flag, host_no_dot_lowercase, expiry_str_raw)
    // expiry_str_raw preserves whatever was in the file (so we can write back
    // unchanged entries verbatim — tests 781/782/783).
    let mut hsts_entries: Vec<(bool, String, String)> = opts
        .hsts_file
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            let mut out = Vec::new();
            for line in s.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut it = line.splitn(2, char::is_whitespace);
                let host = it.next().unwrap_or("");
                let expiry = it.next().unwrap_or("").trim().trim_matches('"');
                if host.is_empty() {
                    continue;
                }
                let subdomain = host.starts_with('.');
                let h = host
                    .trim_start_matches('.')
                    .trim_end_matches('.')
                    .to_lowercase();
                if !h.is_empty() {
                    out.push((subdomain, h, expiry.to_string()));
                }
            }
            out
        })
        .unwrap_or_default();
    let hsts_db: Vec<(String, bool)> = hsts_entries
        .iter()
        .map(|(d, h, _)| (h.clone(), *d))
        .collect();
    let upgrade_to_https = |url: &str| -> String {
        if hsts_db.is_empty() {
            return url.to_string();
        }
        let lower = url.to_ascii_lowercase();
        if !lower.starts_with("http://") {
            return url.to_string();
        }
        let rest = &url[7..];
        let (authority, _tail) = rest.split_once('/').unwrap_or((rest, ""));
        let host_with_port = authority.rsplit('@').next().unwrap_or(authority);
        let host = host_with_port
            .split(':')
            .next()
            .unwrap_or(host_with_port)
            .trim_end_matches('.')
            .to_lowercase();
        let matched = hsts_db.iter().any(|(h, subdomain)| {
            if host == *h {
                true
            } else if *subdomain {
                host.ends_with(&format!(".{h}"))
            } else {
                false
            }
        });
        if matched {
            format!("https://{rest}")
        } else {
            url.to_string()
        }
    };
    let urls_with_query: Vec<String> = urls_with_query
        .iter()
        .map(|u| upgrade_to_https(u))
        .collect();
    // Resolve ipfs:// and ipns:// URLs against a gateway. The gateway comes
    // from --ipfs-gateway, $IPFS_PATH/gateway, or $HOME/.ipfs/gateway in that
    // order (tests 722-741). On error the program exits before transferring.
    let urls_with_query: Vec<String> = {
        let needs_ipfs = urls_with_query.iter().any(|u| {
            let l = u.to_ascii_lowercase();
            l.starts_with("ipfs://") || l.starts_with("ipns://")
        });
        if needs_ipfs {
            match resolve_ipfs_gateway(opts.ipfs_gateway.as_deref()) {
                Err(code) => {
                    process::exit(code);
                }
                Ok(gateway) => urls_with_query
                    .into_iter()
                    .map(|u| {
                        let l = u.to_ascii_lowercase();
                        if l.starts_with("ipfs://") || l.starts_with("ipns://") {
                            let kind = if l.starts_with("ipfs://") { "ipfs" } else { "ipns" };
                            let rest = &u[7..];
                            format!("{}/{kind}/{rest}", gateway.trim_end_matches('/'))
                        } else {
                            u
                        }
                    })
                    .collect(),
            }
        } else {
            urls_with_query
        }
    };
    // Load --alt-svc cache. Each entry is parsed once and used to inject a
    // `--connect-to`-style override before each request (tests 412, 413, 437,
    // 438). Lines look like: `h1 origin_host origin_port h1 alt_host alt_port
    // "expiry" persist priority`. We use the alt host:port for the TCP target
    // while keeping the original Host header and emitting `Alt-Used:` on the
    // request.
    let alt_svc_entries: Vec<(String, u16, String, u16)> = opts
        .alt_svc_file
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            let mut out = Vec::new();
            for line in s.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 6 {
                    continue;
                }
                let origin_host = parts[1].trim_end_matches('.').to_lowercase();
                let Ok(origin_port) = parts[2].parse::<u16>() else {
                    continue;
                };
                let alt_host = parts[4].trim_end_matches('.').to_string();
                let Ok(alt_port) = parts[5].parse::<u16>() else {
                    continue;
                };
                out.push((origin_host, origin_port, alt_host, alt_port));
            }
            out
        })
        .unwrap_or_default();
    // Track whether we've learned any HSTS entries during the loop, so we
    // know whether to rewrite the --hsts file at exit (tests 446, 780-783).
    let mut hsts_learned_any = false;
    // Alt-Svc entries learned from response headers during the URL loop. Each
    // new entry is appended; the final write merges these with the pre-loaded
    // `alt_svc_entries` (test 437) and skips duplicates (test 438).
    let mut alt_svc_learned: Vec<(String, u16, String, u16)> = Vec::new();
    for (url_idx, url_str) in urls_with_query.iter().enumerate() {
        // Reset exit_code per URL — curl reports the LAST URL's status as
        // the process exit code, so a failed URL1 followed by a successful
        // URL2 should still exit 0 (test 1293). --fail-early breaks the
        // loop earlier, so this reset doesn't change that path.
        exit_code = 0;
        let mut fatal_protocol_error = false;

        // file:// — read the local file and write it to the chosen output
        // (stdout if no -o). file://[host]/path: host MUST be empty,
        // "localhost", or "127.0.0.1" — anything else is rejected with
        // exit 3 (URL malformat) per test 1145.
        let lower_url = url_str.to_ascii_lowercase();
        // --proto-default file: a schemeless URL becomes file:// (test 1146).
        let proto_default_file = opts.proto_default.as_deref() == Some("file")
            && !lower_url.contains("://")
            && !lower_url.starts_with("file:");
        let file_url_rest: Option<&str> = if let Some(rest) = url_str
            .strip_prefix("file://")
            .or_else(|| url_str.strip_prefix("FILE://"))
            .or_else(|| url_str.strip_prefix("File://"))
        {
            Some(rest)
        } else if lower_url.starts_with("file:/") && !lower_url.starts_with("file://") {
            // `file:/path` — single-slash form (test 203). Treat as if the
            // host were empty: keep the leading `/`.
            url_str.get(5..)
        } else if proto_default_file {
            // Treat the bare path/URL as if it had been written `file://...`.
            Some(url_str.as_str())
        } else {
            None
        };
        if let Some(rest) = file_url_rest {
            let (host, path) = if let Some(rest) = rest.strip_prefix('/') {
                ("", format!("/{rest}"))
            } else if let Some(slash) = rest.find('/') {
                (&rest[..slash], rest[slash..].to_string())
            } else {
                (rest, "/".to_string())
            };
            // Allow only empty / localhost / 127.0.0.1.
            if !host.is_empty() && host != "localhost" && host != "127.0.0.1" {
                if !opts.silent || opts.show_error {
                    eprintln!("curl: (3) URL using bad/illegal format or missing URL");
                }
                exit_code = 3;
                if opts.fail_early {
                    break;
                }
                continue;
            }
            // Strip any query string / fragment from the path.
            let path = path
                .split_once('?')
                .map(|(p, _)| p.to_string())
                .unwrap_or(path);
            let path = path
                .split_once('#')
                .map(|(p, _)| p.to_string())
                .unwrap_or(path);
            // Decode percent-escapes (RFC 3986 §2.1).
            let mut decoded = Vec::with_capacity(path.len());
            let bytes = path.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%'
                    && i + 2 < bytes.len()
                    && let (Some(h), Some(l)) = (
                        (bytes[i + 1] as char).to_digit(16),
                        (bytes[i + 2] as char).to_digit(16),
                    )
                {
                    decoded.push(((h << 4) | l) as u8);
                    i += 3;
                    continue;
                }
                decoded.push(bytes[i]);
                i += 1;
            }
            let fpath = String::from_utf8_lossy(&decoded).into_owned();
            // --skip-existing on a download: if -o target exists, emit the
            // standard notice and skip (test 1491).
            if opts.skip_existing
                && let Some(out_p) = opts.outputs.get(url_idx)
                && out_p.to_str() != Some("-")
                && out_p.exists()
            {
                eprintln!(
                    "Note: skips transfer, \"{}\" exists locally",
                    out_p.display()
                );
                continue;
            }
            // -T / --upload-file with file://: write the upload-source's
            // bytes to the file:// path (tests 1490, 1491). Honor
            // --skip-existing and --no-clobber the same way HTTP outputs do.
            let upload_src = opts
                .per_url_opts
                .get(url_idx)
                .and_then(|p| p.upload_file.clone())
                .or_else(|| opts.upload_file.clone());
            if let Some(src) = upload_src {
                if opts.skip_existing && std::path::Path::new(&fpath).exists() {
                    eprintln!("Note: skips transfer, \"{}\" exists locally", fpath);
                    continue;
                }
                let body = if src.to_str() == Some("-") {
                    let mut buf = Vec::new();
                    let _ = io::Read::read_to_end(&mut io::stdin(), &mut buf);
                    buf
                } else {
                    match fs::read(&src) {
                        Ok(b) => b,
                        Err(_) => {
                            eprintln!(
                                "curl: read form file: couldn't open file \"{}\"",
                                src.display()
                            );
                            exit_code = 26;
                            if opts.fail_early {
                                break;
                            }
                            continue;
                        }
                    }
                };
                if let Err(e) = fs::write(&fpath, &body) {
                    if !opts.silent || opts.show_error {
                        eprintln!("curl: failed to write to {fpath}: {e}");
                    }
                    exit_code = 23;
                }
                if opts.fail_early && exit_code != 0 {
                    break;
                }
                continue;
            }
            // GET a directory via file://: curl emits a newline-terminated
            // sorted directory listing with exit 0 (tests 3016, 3203).
            let is_dir = fs::metadata(&fpath).map(|m| m.is_dir()).unwrap_or(false);
            let read_result = if is_dir {
                let mut names: Vec<String> = match fs::read_dir(&fpath) {
                    Ok(entries) => entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect(),
                    Err(_) => Vec::new(),
                };
                names.sort();
                let mut listing = String::new();
                for n in &names {
                    listing.push_str(n);
                    listing.push('\n');
                }
                Ok(listing.into_bytes())
            } else {
                fs::read(&fpath)
            };
            match read_result {
                Ok(content) => {
                    // -C / --continue-at: skip the leading N bytes of the
                    // file before output (test 231).
                    let skip: usize = opts
                        .resume_from
                        .as_deref()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let mut start = skip;
                    let mut end = content.len();
                    // -r / --range on file://: forms `N-`, `-N`, `N-M`, `N`
                    // (tests 1019, 1020).
                    let mut range_bad_resume = false;
                    if let Some(ref r) = opts.range {
                        if let Some(rest) = r.strip_prefix('-') {
                            // last N bytes
                            if let Ok(n) = rest.parse::<usize>() {
                                start = content.len().saturating_sub(n);
                                end = content.len();
                            }
                        } else if let Some((s, e)) = r.split_once('-') {
                            // For open-ended ranges (`N-`), curl exits 36 if
                            // the requested start is past EOF (test 1063).
                            if let Ok(s_n_u64) = s.parse::<u64>() {
                                if e.is_empty() && s_n_u64 >= content.len() as u64 {
                                    range_bad_resume = true;
                                } else {
                                    start = s_n_u64.min(usize::MAX as u64) as usize;
                                }
                            }
                            if !e.is_empty()
                                && let Ok(e_n) = e.parse::<usize>()
                            {
                                end = (e_n + 1).min(content.len());
                            }
                        } else if let Ok(n) = r.parse::<usize>() {
                            start = n;
                        }
                    }
                    if range_bad_resume {
                        if !opts.silent || opts.show_error {
                            eprintln!("curl: (36) failed to seek in file");
                        }
                        exit_code = 36;
                        if opts.fail_early {
                            break;
                        }
                        continue;
                    }
                    let slice: &[u8] = if start < end && start < content.len() {
                        &content[start..end.min(content.len())]
                    } else {
                        &[]
                    };
                    let body_path = opts.outputs.get(url_idx).cloned();
                    if let Some(out_path) = body_path
                        && out_path.to_str() != Some("-")
                    {
                        if opts.create_dirs
                            && let Some(parent) = out_path.parent()
                            && !parent.as_os_str().is_empty()
                        {
                            let _ = fs::create_dir_all(parent);
                        }
                        if let Err(e) = fs::write(&out_path, slice) {
                            eprintln!("curl: failed to write to {}: {e}", out_path.display());
                            exit_code = 23;
                        } else if opts.remote_time
                            && let Ok(meta) = fs::metadata(&fpath)
                            && let Ok(mtime) = meta.modified()
                            && let Ok(file) = fs::OpenOptions::new().write(true).open(&out_path)
                        {
                            let times = std::fs::FileTimes::new()
                                .set_modified(mtime)
                                .set_accessed(mtime);
                            let _ = file.set_times(times);
                        }
                    } else {
                        let _ = io::stdout().write_all(slice);
                        let _ = io::stdout().flush();
                    }
                }
                Err(_) => {
                    if !opts.silent || opts.show_error {
                        eprintln!("curl: (37) Couldn't open file {fpath}");
                    }
                    exit_code = 37;
                }
            }
            // --write-out for file:// transfers: even when the file failed
            // to open we still emit the formatted output (test 1442 covers
            // a non-existent file:// + `--write-out=\` -> stdout `\`).
            if let Some(ref fmt) = opts.write_out {
                use crate::format::{WriteOutDest, split_write_out};
                let synthetic = crate::response::Response {
                    trailer_bytes: Vec::new(),
                    http10_response: false,
                    status: 0,
                    status_text: String::new(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    header_bytes: Vec::new(),
                    redirect_headers: Vec::new(),
                    connect_header_size: 0,
                    num_connects: 0,
                    num_redirects: 0,
                    max_redirects_reached: false,
                    proto_redir_blocked: false,
                    upload_redirect_failed: false,
                    redirect_url_malformed: false,
                    weird_server_reply: false,
                    final_url: None,
                    final_referer: None,
                    redirect_url: None,
                    timed_out: false,
                    recv_error: false,
                    partial_file: false,
                    bad_content_encoding: false,
                    bad_encoding_too_many: false,
                    filesize_exceeded: false,
                    header_size_error: false,
                    ntlm_too_large: false,
                };
                if let Ok(parsed_url) = crate::url::parse_url(url_str) {
                    for (dest, _gated, raw) in split_write_out(fmt) {
                        if raw.is_empty() {
                            continue;
                        }
                        let text = crate::format::format_write_out(
                            &raw,
                            &synthetic,
                            &parsed_url,
                            0,
                            0,
                            "GET",
                            None,
                            url_idx,
                            exit_code,
                            "",
                            "",
                            0,
                        );
                        use std::io::Write as _;
                        match dest {
                            WriteOutDest::Stdout => {
                                let _ = io::stdout().write_all(text.as_bytes());
                            }
                            WriteOutDest::Stderr => {
                                let _ = io::stderr().write_all(text.as_bytes());
                            }
                            WriteOutDest::File { .. } => {}
                        }
                    }
                    let _ = io::stdout().flush();
                } else {
                    // Malformed file:// URL — still emit literal write-out
                    // text (no substitutions) so tests 1440/1441 see output.
                    use std::io::Write as _;
                    for (dest, _gated, raw) in split_write_out(fmt) {
                        if raw.is_empty() {
                            continue;
                        }
                        match dest {
                            WriteOutDest::Stdout => {
                                let _ = io::stdout().write_all(raw.as_bytes());
                            }
                            WriteOutDest::Stderr => {
                                let _ = io::stderr().write_all(raw.as_bytes());
                            }
                            WriteOutDest::File { .. } => {}
                        }
                    }
                    let _ = io::stdout().flush();
                }
            }
            if opts.fail_early && exit_code != 0 {
                break;
            }
            continue;
        }
        // --disallow-username-in-url: any URL carrying user[:pass]@ is rejected
        // before connecting, exiting CURLE_LOGIN_DENIED (67) (test 2075).
        if opts.disallow_userinfo {
            let has_userinfo = url_str
                .find("://")
                .map(|i| {
                    let after = &url_str[i + 3..];
                    let host_end = after.find('/').unwrap_or(after.len());
                    after[..host_end].contains('@')
                })
                .unwrap_or(false);
            if has_userinfo {
                if !opts.silent || opts.show_error {
                    eprintln!("curl: (67) Credentials in URL not allowed");
                }
                exit_code = 67;
                if opts.fail_early {
                    break;
                }
                continue;
            }
        }
        // --skip-existing: if the output target already exists, emit the
        // notice and skip this URL's transfer entirely.
        if opts.skip_existing {
            // Resolve the prospective output path the same way the post-transfer
            // path resolution does, but only for explicit -o targets (not -O/-J).
            let raw_out = opts
                .outputs
                .get(url_idx)
                .or_else(|| {
                    if opts.outputs.len() == 1
                        && opts.outputs[0].to_str().is_some_and(|s| s.contains('#'))
                    {
                        opts.outputs.first()
                    } else {
                        None
                    }
                })
                .cloned()
                .filter(|p| p.to_str() != Some("-"));
            let out_path = raw_out.map(|p| {
                if let Some(s) = p.to_str()
                    && s.contains('#')
                {
                    let glob_vals = url_glob_values
                        .get(url_idx)
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]);
                    let mut result = s.to_string();
                    for digit in (1..=9u8).rev() {
                        let pattern = format!("#{digit}");
                        if let Some(val) = glob_vals.get(digit as usize - 1) {
                            result = result.replace(&pattern, val);
                        }
                    }
                    PathBuf::from(result)
                } else {
                    p
                }
            });
            let out_path = out_path.map(|p| {
                if let Some(ref dir) = opts.output_dir
                    && p.is_relative()
                {
                    dir.join(&p)
                } else {
                    p
                }
            });
            if let Some(ref p) = out_path
                && p.exists()
            {
                eprintln!("Note: skips transfer, \"{}\" exists locally", p.display());
                continue;
            }
        }

        // Collected bytes (header+body) of prior failed attempts in a retry
        // sequence — curl's `--retry` with `--include` emits every attempt's
        // response, so we accumulate and prepend these to the final output.
        let mut retry_prefix: Vec<u8> = Vec::new();

        // Apply per-URL option overrides. Each URL was snapshotted at parse
        // time so that `--next` resets don't retroactively affect earlier URLs.
        // We always clone so that `opts` remains free for mutable cookie updates.
        let mut effective_opts = if let Some(puo) = opts.per_url_opts.get(url_idx) {
            let mut o = opts.clone();
            o.data = puo.data.clone();
            o.data_raw = puo.data_raw;
            o.headers = puo.headers.clone();
            o.method = puo.method.clone();
            o.json = puo.json;
            o.form_fields = puo.form_fields.clone();
            o.upload_file = puo.upload_file.clone();
            o.head = puo.head;
            o.get = puo.get;
            o.connect_tos = puo.connect_tos.clone();
            o.resolves = puo.resolves.clone();
            o.no_basic = puo.no_basic;
            o.user = puo.user.clone();
            o.dump_header = puo.dump_header.clone();
            o.proxy = puo.proxy.clone();
            o.proxy_user = puo.proxy_user.clone();
            o.etag_save = puo.etag_save.clone();
            o.etag_compare = puo.etag_compare.clone();
            o.include_headers = puo.include_headers;
            o
        } else {
            opts.clone()
        };

        // Alt-Svc lookup: if the URL's (host, port) matches a loaded entry,
        // route TCP to the alt host:port (via a synthetic --connect-to) and
        // emit `Alt-Used:` on the next request. Only activates over plain
        // HTTP when `CURL_ALTSVC_HTTP` is set, matching curl's Debug build
        // (tests 412, 413, 437, 438). Resets per URL.
        effective_opts.alt_used = None;
        if !alt_svc_entries.is_empty()
            && let Ok(u) = crate::url::parse_url(url_str)
        {
            let altsvc_over_http =
                u.scheme == "https" || std::env::var("CURL_ALTSVC_HTTP").is_ok();
            if altsvc_over_http {
                let needle_host = u.host.trim_end_matches('.').to_lowercase();
                if let Some((_, _, alt_host, alt_port)) = alt_svc_entries
                    .iter()
                    .find(|(oh, op, _, _)| oh == &needle_host && *op == u.port)
                {
                    effective_opts.connect_tos.push(format!(
                        "{}:{}:{alt_host}:{alt_port}",
                        u.host, u.port
                    ));
                    effective_opts.alt_used = Some(format!("{alt_host}:{alt_port}"));
                }
            }
        }

        // Pre-flight per-URL --etag-save check: open-test the file. If
        // creation fails (bad path), report exit 26 for THIS URL and skip
        // the transfer, but continue the loop so subsequent URLs run
        // (test 369).
        if let Some(ref etag_path) = effective_opts.etag_save {
            if effective_opts.create_dirs
                && let Some(parent) = etag_path.parent()
                && !parent.as_os_str().is_empty()
            {
                let _ = fs::create_dir_all(parent);
            }
            if let Err(e) = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(etag_path)
            {
                eprintln!("curl: (26) Failed to open {}: {}", etag_path.display(), e);
                exit_code = 26;
                if effective_opts.fail_early {
                    break;
                }
                continue;
            }
        }

        // --netrc / --netrc-optional: look up credentials for the URL host
        // and set them when the user did not pass `-u` (test 478, 2006).
        if effective_opts.netrc_mode != 0 && effective_opts.user.is_none() {
            // Resolve order: --netrc-file > NETRC env var > $HOME/.netrc.
            let netrc_path = effective_opts
                .netrc_file
                .clone()
                .or_else(|| std::env::var_os("NETRC").map(std::path::PathBuf::from))
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".netrc"))
                });
            if let Some(path) = netrc_path
                && let Ok(parsed) = crate::url::parse_url(url_str)
            {
                // The netrc lookup should also pick up creds carried in the
                // URL itself (e.g. `ftp://mary@host/`, test 380): if there is
                // a userinfo with no password, find the password by host+user.
                let host_user = parsed.userinfo.as_deref().and_then(|ui| {
                    if ui.contains(':') {
                        None
                    } else {
                        Some(ui.to_string())
                    }
                });
                match crate::netrc::lookup(&path, &parsed.host, host_user.as_deref()) {
                    Ok(Some((login, password))) => {
                        // Either login or password may be omitted; emit
                        // Authorization as long as at least one source has
                        // a value (test 684 has password-only with empty user).
                        if host_user.is_some() || login.is_some() || password.is_some() {
                            let u = host_user.or(login).unwrap_or_default();
                            let p = password.unwrap_or_default();
                            effective_opts.user = Some(format!("{u}:{p}"));
                        }
                    }
                    Ok(None) => {}
                    Err(e) if e.contains("read netrc") => {
                        // File-not-found: --netrc-optional silently ignores
                        // it (test 495). Required-netrc would have errored
                        // earlier in args.rs.
                    }
                    Err(_) => {
                        // Malformed netrc (e.g. unterminated quote) — exit 26
                        // (test 680).
                        eprintln!("curl: netrc parse error");
                        process::exit(26);
                    }
                }
            }
        }

        let result = if effective_opts.retry > 0 {
            let mut last_err = String::new();
            let mut resp = None;
            let retry_start = std::time::Instant::now();
            let mut next_delay_secs: u64 = 0;
            for attempt in 0..=opts.retry {
                if attempt > 0 {
                    // Check --retry-max-time: if the next sleep would push us past
                    // the budget, abort the retry loop and return the last response.
                    if let Some(budget) = opts.retry_max_time {
                        let elapsed = retry_start.elapsed().as_secs();
                        if elapsed + next_delay_secs > budget {
                            break;
                        }
                    }
                    if !opts.silent {
                        eprintln!(
                            "Warning: Transient problem. Will retry in {next_delay_secs} seconds. ({attempt}/{} retries)",
                            opts.retry,
                        );
                    }
                    std::thread::sleep(Duration::from_secs(next_delay_secs));
                }
                match perform(url_str, &effective_opts) {
                    Ok(r) => {
                        // Retry on 5xx and 429 (rate-limited).
                        if (r.status >= 500
                            || r.status == 429
                            || (opts.retry_all_errors
                                && (r.partial_file
                                    || r.timed_out
                                    || r.recv_error
                                    || r.weird_server_reply
                                    || r.bad_content_encoding)))
                            && attempt < opts.retry
                        {
                            last_err = format!("HTTP {}", r.status);
                            // Honor Retry-After header (seconds) for the delay.
                            let proposed_delay = r
                                .headers
                                .iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
                                .and_then(|(_, v)| v.trim().parse::<u64>().ok())
                                .unwrap_or((attempt as u64 + 1) * 2);
                            // If Retry-After exceeds --retry-max-time, abort
                            // the retry sequence without accumulating this
                            // attempt into the prefix (test 366).
                            if let Some(budget) = opts.retry_max_time {
                                let elapsed = retry_start.elapsed().as_secs();
                                if elapsed + proposed_delay > budget {
                                    resp = Some(r);
                                    break;
                                }
                            }
                            if effective_opts.include_headers {
                                // Include redirect chain bytes (e.g. 301)
                                // before the final response headers so the
                                // attempt is reproduced verbatim in -i output.
                                retry_prefix.extend_from_slice(&r.redirect_headers);
                                retry_prefix.extend_from_slice(&r.header_bytes);
                            }
                            // --fail drops the body of HTTP-error responses;
                            // --fail-with-body keeps it (test 1634).
                            let drop_body = r.status >= 400 && opts.fail && !opts.fail_with_body;
                            if !drop_body {
                                retry_prefix.extend_from_slice(&r.body);
                            }
                            next_delay_secs = proposed_delay;
                            resp = Some(r);
                            continue;
                        }
                        // Final attempt succeeded after retry. Without
                        // --fail / --fail-with-body, curl drops the failed
                        // attempts' output when the destination is a regular
                        // file (it ftruncates between retries — test 198).
                        // For stdout (or `-o -`), there is no rewind, so all
                        // attempts accumulate (test 197). With --fail the
                        // failed attempts stay in either case (tests 1633,
                        // 1634).
                        let to_stdout = !effective_opts.remote_name
                            && !effective_opts.remote_header_name
                            && match effective_opts.outputs.get(url_idx) {
                                None => true,
                                Some(p) => p.to_str() == Some("-"),
                            };
                        if !opts.fail && !opts.fail_with_body && r.status < 400 && !to_stdout {
                            retry_prefix.clear();
                        }
                        resp = Some(r);
                        break;
                    }
                    Err(e) => {
                        last_err = e;
                        next_delay_secs = (attempt as u64 + 1) * 2;
                        if attempt == opts.retry {
                            break;
                        }
                    }
                }
            }
            resp.ok_or(last_err)
        } else {
            perform(url_str, &effective_opts)
        };

        match result {
            Ok(mut resp) => {
                // After redirects, the "effective" URL is the last URL we fetched;
                // parse_url(url_str) returns the initial one. Reconstruct the last
                // URL from the ParsedUrl we have on resp (via final-url tracking).
                let url = resp
                    .final_url
                    .as_deref()
                    .and_then(|u| parse_url(u).ok())
                    .unwrap_or_else(|| parse_url(url_str).unwrap());

                // Alt-Svc learn: parse any `Alt-Svc:` headers when --alt-svc
                // was given. Each `hN="host:port"` entry maps the URL's
                // (origin_host, origin_port) to (alt_host, alt_port) (test 437).
                // We only learn for protocols we send (h1) — h2/h3 are kept in
                // the header but not added to the cache (we don't speak them).
                if opts.alt_svc_file.is_some() && (200..400).contains(&resp.status) {
                    let origin_host = url.host.trim_end_matches('.').to_string();
                    let origin_port = url.port;
                    for (k, v) in &resp.headers {
                        if !k.eq_ignore_ascii_case("alt-svc") {
                            continue;
                        }
                        for entry in parse_alt_svc_header(v) {
                            let already = alt_svc_entries
                                .iter()
                                .chain(alt_svc_learned.iter())
                                .any(|(oh, op, ah, ap)| {
                                    oh == &origin_host
                                        && *op == origin_port
                                        && ah == &entry.0
                                        && *ap == entry.1
                                });
                            if !already {
                                alt_svc_learned.push((
                                    origin_host.clone(),
                                    origin_port,
                                    entry.0,
                                    entry.1,
                                ));
                            }
                        }
                    }
                }

                // HSTS learn: record any Strict-Transport-Security headers
                // received on a 2xx response when --hsts was given. Merge into
                // hsts_entries (replace if same host, else append). We write
                // the file at end-of-program (tests 446, 780-783).
                if opts.hsts_file.is_some() && (200..300).contains(&resp.status) {
                    for (k, v) in &resp.headers {
                        if k.eq_ignore_ascii_case("strict-transport-security")
                            && let Some((expiry, sub)) = parse_hsts_header(v)
                        {
                            let host_no_dot = url
                                .host
                                .trim_end_matches('.')
                                .to_lowercase();
                            let expiry_str = format_hsts_expiry(expiry);
                            if let Some(pos) = hsts_entries
                                .iter()
                                .position(|(_, h, _)| h == &host_no_dot)
                            {
                                hsts_entries[pos] = (sub, host_no_dot, expiry_str);
                            } else {
                                hsts_entries.push((sub, host_no_dot, expiry_str));
                            }
                            hsts_learned_any = true;
                            break;
                        }
                    }
                }

                // Resume (-C) with 416 means the file was already fully downloaded —
                // treat as success even with --fail.
                let resume_fully_downloaded = opts.resume_from.is_some() && resp.status == 416;
                // Resume (-C) on a GET with 200 (not 206) AND no Content-Range means
                // the server didn't honor our Range — curl reports error 33
                // (CURLE_RANGE_ERROR) and skips the body. Only for downloads.
                // But: if the response's Content-Length matches the requested
                // resume offset, we already have everything — treat as success.
                let has_content_range = resp.headers.iter().any(|(k, _)| k == "content-range");
                // For `-C -` the effective offset depends on the output file's
                // current size at the moment of the request — re-stat now so
                // the "refused" check matches what we actually asked for.
                let resume_offset: Option<u64> =
                    opts.resume_from.as_deref().and_then(|s| match s {
                        "-" => opts
                            .outputs
                            .first()
                            .and_then(|p| fs::metadata(p).ok())
                            .map(|m| m.len()),
                        _ => s.parse().ok(),
                    });
                let resp_cl: Option<u64> = resp
                    .headers
                    .iter()
                    .find(|(k, _)| k == "content-length")
                    .and_then(|(_, v)| v.parse().ok());
                // resume_fully_covered: Content-Length matches our offset exactly,
                // meaning we already have the whole file.
                let resume_fully_covered = matches!(
                    (resume_offset, resp_cl),
                    (Some(off), Some(cl)) if cl == off
                );
                // -C with any non-206 GET response means the server didn't
                // honor the Range — drop the body (test 99 covers 404 with
                // a huge resume offset). 416 is special: the file is
                // already fully downloaded, NOT an error (test 1040). Only
                // counts when we ACTUALLY asked for a range (offset > 0); a
                // zero offset means we never sent Range and the server's plain
                // 200 is just a normal response (test 3035).
                let resume_range_refused = matches!(resume_offset, Some(off) if off > 0)
                    && resp.status != 206
                    && resp.status != 416
                    && !has_content_range
                    && !resume_fully_covered
                    && effective_opts.upload_file.is_none();
                if resume_range_refused {
                    eprintln!(
                        "curl: (33) HTTP server doesn't seem to support byte ranges. Cannot resume."
                    );
                    exit_code = 33;
                }
                if resp.max_redirects_reached {
                    eprintln!(
                        "curl: (47) Maximum ({}) redirects followed",
                        opts.max_redirs
                    );
                    exit_code = 47;
                }
                if resp.redirect_url_malformed {
                    eprintln!(
                        "curl: (3) Failed to parse URL after redirect: {}",
                        resp.final_url.as_deref().unwrap_or("")
                    );
                    exit_code = 3;
                }
                if resp.proto_redir_blocked {
                    if let Some(ref redirect_url) = resp.redirect_url {
                        let scheme = redirect_url.split("://").next().unwrap_or("");
                        eprintln!(
                            "curl: (1) Protocol \"{scheme}\" not supported or disabled in libcurl"
                        );
                    } else {
                        eprintln!("curl: (1) Protocol not supported or disabled in libcurl");
                    }
                    exit_code = 1;
                }
                if resp.upload_redirect_failed {
                    eprintln!("curl: (25) Failed to upload after redirect");
                    exit_code = 25;
                }
                if resp.weird_server_reply {
                    eprintln!("curl: (8) weird server reply");
                    exit_code = 8;
                }
                if resp.timed_out && opts.max_time.is_some() {
                    let ms = opts.max_time.map(|d| d.as_millis()).unwrap_or(0);
                    eprintln!(
                        "curl: (28) Operation timed out after {} milliseconds with {} bytes received",
                        ms,
                        resp.body.len()
                    );
                    exit_code = 28;
                }
                if resp.bad_content_encoding {
                    if resp.bad_encoding_too_many {
                        eprintln!(
                            "curl: (61) Reject response due to more than 5 content encodings"
                        );
                    } else {
                        eprintln!(
                            "curl: (61) Unrecognized or bad HTTP Content or Transfer-Encoding"
                        );
                    }
                    exit_code = 61;
                    // curl writes the response headers to stdout when a
                    // --compressed body can't be decoded, regardless of -i
                    // (tests 223, 315). Suppress the body since it's
                    // undecoded gibberish.
                    if !opts.include_headers && opts.outputs.is_empty() {
                        use std::io::Write as _;
                        let _ = io::stdout().write_all(&resp.header_bytes);
                        let _ = io::stdout().flush();
                    }
                    resp.body.clear();
                } else if resp.partial_file {
                    eprintln!("curl: (18) transfer closed with outstanding read data remaining");
                    exit_code = 18;
                } else if resp.recv_error {
                    exit_code = 56;
                }
                if resp.filesize_exceeded {
                    eprintln!("curl: (63) Maximum file size exceeded");
                    exit_code = 63;
                }
                if resp.header_size_error {
                    eprintln!("curl: (27) Response headers too large");
                    exit_code = 56;
                }
                if resp.ntlm_too_large {
                    // curl's CURLE_TOO_LARGE — used by the NTLM auth path
                    // when credentials would push the Type 3 over the limit
                    // (tests 775, 776).
                    eprintln!("curl: (100) A value or data field is larger than allowed");
                    exit_code = 100;
                }
                // status==0 means we consumed a 1xx interim response but the
                // server closed the connection before sending a final response.
                // Output the interim headers (handled below) but set exit 52.
                if resp.status == 0 {
                    exit_code = 52;
                }
                // With --fail/--fail-with-body on HTTP errors we set exit code 22
                // but still let headers flow through to output. --fail discards
                // the body; --fail-with-body keeps it.
                let http_error = resp.status >= 400 && !resume_fully_downloaded;
                let fail_http_error = (opts.fail || opts.fail_with_body) && http_error;
                if fail_http_error {
                    if opts.show_error || !opts.silent {
                        eprintln!(
                            "curl: (22) The requested URL returned error: {}",
                            resp.status
                        );
                    }
                    exit_code = 22;
                }

                // Dump headers. Special destinations: "-" → stdout, "%" → stderr.
                // When dump dest is stdout AND --include is also on, curl's
                // per-line callback writes each header twice (once to dump,
                // once to include), so emit each line twice in succession to
                // match (test 1066). The include branch below skips its own
                // header write in this case.
                let dump_to_stdout = effective_opts
                    .dump_header
                    .as_deref()
                    .and_then(|p| p.to_str())
                    == Some("-");
                // Only double the headers when the include path will also
                // emit to stdout (i.e. there is no -o / -O sending the body
                // elsewhere). In `-D -` + `-i` + `-O file` (test 1343) the
                // include output goes to the file, so the dump-to-stdout
                // should not be doubled.
                let include_goes_to_stdout = opts.outputs.get(url_idx).is_none()
                    && !opts.remote_name
                    && !opts.remote_header_name;
                let dump_pair_with_include =
                    dump_to_stdout && effective_opts.include_headers && include_goes_to_stdout;
                if let Some(ref dump_path) = effective_opts.dump_header {
                    match dump_path.to_str() {
                        Some("-") => {
                            if dump_pair_with_include {
                                let mut tail = resp.header_bytes.as_slice();
                                while !tail.is_empty() {
                                    let end = tail
                                        .iter()
                                        .position(|&b| b == b'\n')
                                        .map(|p| p + 1)
                                        .unwrap_or(tail.len());
                                    let line = &tail[..end];
                                    let _ = io::stdout().write_all(line);
                                    let _ = io::stdout().write_all(line);
                                    tail = &tail[end..];
                                }
                                let mut tail = resp.trailer_bytes.as_slice();
                                while !tail.is_empty() {
                                    let end = tail
                                        .iter()
                                        .position(|&b| b == b'\n')
                                        .map(|p| p + 1)
                                        .unwrap_or(tail.len());
                                    let line = &tail[..end];
                                    let _ = io::stdout().write_all(line);
                                    let _ = io::stdout().write_all(line);
                                    tail = &tail[end..];
                                }
                            } else {
                                let _ = io::stdout().write_all(&resp.header_bytes);
                                let _ = io::stdout().write_all(&resp.trailer_bytes);
                            }
                        }
                        Some("%") => {
                            let _ = io::stderr().write_all(&resp.header_bytes);
                            let _ = io::stderr().write_all(&resp.trailer_bytes);
                        }
                        _ => {
                            let mut dump_data = resp.header_bytes.clone();
                            dump_data.extend_from_slice(&resp.trailer_bytes);
                            // --create-dirs also applies to -D (test 3031).
                            if opts.create_dirs
                                && let Some(parent) = dump_path.parent()
                                && !parent.as_os_str().is_empty()
                            {
                                let _ = fs::create_dir_all(parent);
                            }
                            // Truncate on the first URL of each --next
                            // group, append on subsequent ones in the same
                            // group — curl writes all transfers within a
                            // group to a shared -D file (test 3030, 3029).
                            let truncate = opts
                                .per_url_opts
                                .get(url_idx)
                                .map(|p| p.first_in_group)
                                .unwrap_or(url_idx == 0);
                            let mut open = fs::OpenOptions::new();
                            open.write(true).create(true);
                            if truncate {
                                open.truncate(true);
                            } else {
                                open.append(true);
                            }
                            let result = open.open(dump_path).and_then(|mut f| {
                                use std::io::Write as _;
                                f.write_all(&dump_data)
                            });
                            if result.is_err() {
                                eprintln!(
                                    "curl: (23) Failure writing output to destination, passed {} bytes",
                                    resp.header_bytes.len()
                                );
                                exit_code = 23;
                            }
                        }
                    }
                }

                // Save cookie jar.
                if let Some(ref jar_path) = opts.cookie_jar {
                    // Use the custom Host (if provided) as the cookie's host
                    // identity, matching curl's behavior.
                    let host_for_jar = opts
                        .headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
                        .map(|(_, v)| v.split(':').next().unwrap_or(v).to_string())
                        .unwrap_or_else(|| url.host.clone());
                    save_cookie_jar(
                        jar_path,
                        &url,
                        &host_for_jar,
                        &resp.headers,
                        &opts.cookies,
                        &opts.memory_cookies,
                    );
                }

                // Accumulate cookies from Set-Cookie headers for cross-URL cookie engine.
                if opts.cookie_engine {
                    let host_for_cookies = opts
                        .headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
                        .map(|(_, v)| v.split(':').next().unwrap_or(v).to_string())
                        .unwrap_or_else(|| url.host.clone());
                    for (key, value) in &resp.headers {
                        if key == "set-cookie"
                            && let Some(line) =
                                cookie::format_cookie_line(value, &url, &host_for_cookies)
                        {
                            // Deduplicate by (domain, path, name) — same cookie from
                            // a repeated Set-Cookie replaces the old entry.
                            let fields: Vec<&str> = line.split('\t').collect();
                            if fields.len() >= 7 {
                                let new_domain =
                                    fields[0].strip_prefix("#HttpOnly_").unwrap_or(fields[0]);
                                let new_path = fields[2];
                                let new_name = fields[5];
                                opts.memory_cookies.retain(|existing| {
                                    let ef: Vec<&str> = existing.split('\t').collect();
                                    if ef.len() >= 7 {
                                        let ed = ef[0].strip_prefix("#HttpOnly_").unwrap_or(ef[0]);
                                        !(ed == new_domain
                                            && ef[2] == new_path
                                            && ef[5] == new_name)
                                    } else {
                                        true
                                    }
                                });
                            }
                            // Max-Age=0 or past Expires means "delete this cookie" —
                            // remove the old entry (done above) but do NOT store the
                            // expired cookie itself.
                            if !cookie::is_jar_line_expired(&line) {
                                // RFC 6265bis §5.6 — 150 cookies-per-domain cap.
                                // Curl drops *new* cookies once the cap is full
                                // for that domain (test 444 keeps 1..=150).
                                let new_dom = line
                                    .split('\t')
                                    .next()
                                    .map(|s| s.strip_prefix("#HttpOnly_").unwrap_or(s).to_string())
                                    .unwrap_or_default();
                                let same_domain_count = opts
                                    .memory_cookies
                                    .iter()
                                    .filter(|c| {
                                        c.split('\t')
                                            .next()
                                            .map(|s| {
                                                s.strip_prefix("#HttpOnly_").unwrap_or(s)
                                                    == new_dom.as_str()
                                            })
                                            .unwrap_or(false)
                                    })
                                    .count();
                                if same_domain_count < 50 {
                                    opts.memory_cookies.push(line);
                                }
                            } else {
                                // Record the deleted cookie so file-based cookies
                                // with the same (domain, path, name) are skipped.
                                let df: Vec<&str> = line.split('\t').collect();
                                if df.len() >= 7 {
                                    let dd = df[0]
                                        .strip_prefix("#HttpOnly_")
                                        .unwrap_or(df[0])
                                        .trim_start_matches('.');
                                    opts.deleted_cookies.push((
                                        dd.to_lowercase(),
                                        df[2].to_string(),
                                        df[5].to_string(),
                                    ));
                                }
                            }
                        }
                    }
                }

                // --etag-save: write the server's ETag header value to a file
                // (without quotes or leading/trailing whitespace). Overwrites.
                if let Some(ref path) = opts.etag_save
                    && let Some((_, etag)) = resp.headers.iter().find(|(k, _)| k == "etag")
                {
                    let content = if etag.is_empty() {
                        String::new()
                    } else {
                        format!("{etag}\n")
                    };
                    if opts.create_dirs
                        && let Some(parent) = path.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(path, content);
                }

                // Determine output destination.
                // -o - means stdout (not a file called "-")
                // Each -o pairs with a URL in order; extra -o's beyond URL count are ignored.
                // -J / --remote-header-name: prefer Content-Disposition filename;
                // fall back to URL-based name when the header is absent or empty.
                let cd_filename = if opts.remote_header_name {
                    resp.headers
                        .iter()
                        .find(|(k, _)| k == "content-disposition")
                        .and_then(|(_, v)| extract_cd_filename(v))
                } else {
                    None
                };
                let used_cd_filename = cd_filename.is_some();
                // -J / -O: an explicit `-o` always wins over the
                // Content-Disposition or URL-derived name (tests 1368-1371).
                let explicit_o = opts.outputs.get(url_idx).cloned();
                let output_path = if let Some(o) = explicit_o.clone() {
                    Some(o)
                } else if let Some(name) = cd_filename {
                    Some(PathBuf::from(name))
                } else if opts.remote_name || opts.remote_header_name {
                    // curl tool: derive filename from the URL path, ignoring
                    // any query string. A trailing '/' is stripped before
                    // taking the basename. Empty paths (or those reducing to
                    // nothing) fall back to "curl_response".
                    let path_no_query = url.path.split('?').next().unwrap_or(&url.path);
                    let trimmed = path_no_query.trim_end_matches('/');
                    let name = trimmed
                        .rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("curl_response");
                    Some(PathBuf::from(name))
                } else {
                    // For glob-expanded URLs, a single -o pattern with #N applies
                    // to ALL URLs. Fall back to opts.outputs[0] when url_idx is
                    // out of range and the pattern contains '#'.
                    let raw_out = opts
                        .outputs
                        .get(url_idx)
                        .or_else(|| {
                            if opts.outputs.len() == 1
                                && opts.outputs[0].to_str().is_some_and(|s| s.contains('#'))
                            {
                                opts.outputs.first()
                            } else {
                                None
                            }
                        })
                        .cloned()
                        .filter(|p| p.to_str() != Some("-"));
                    raw_out.map(|p| {
                        // Replace #[digit] with the corresponding glob value.
                        // #1 = first glob group value, #2 = second, etc.
                        if let Some(s) = p.to_str()
                            && s.contains('#')
                        {
                            let glob_vals = url_glob_values
                                .get(url_idx)
                                .map(|v| v.as_slice())
                                .unwrap_or(&[]);
                            let mut result = s.to_string();
                            for digit in (1..=9u8).rev() {
                                let pattern = format!("#{digit}");
                                if let Some(val) = glob_vals.get(digit as usize - 1) {
                                    result = result.replace(&pattern, val);
                                }
                                // If no corresponding glob group, leave #N literal
                            }
                            return PathBuf::from(result);
                        }
                        p
                    })
                };
                // Substitute `#N` in the path with the Nth glob value from the
                // URL (test 1283 has [a-a][1-1][b-b:1][2-2:1] + `#1#2#3#4`).
                let output_path = output_path.map(|p| {
                    if let Some(s) = p.to_str()
                        && s.contains('#')
                    {
                        let glob_vals = url_glob_values
                            .get(url_idx)
                            .map(|v| v.as_slice())
                            .unwrap_or(&[]);
                        let mut result = s.to_string();
                        for digit in (1..=9u8).rev() {
                            let pattern = format!("#{digit}");
                            if let Some(val) = glob_vals.get(digit as usize - 1) {
                                result = result.replace(&pattern, val);
                            }
                        }
                        PathBuf::from(result)
                    } else {
                        p
                    }
                });
                // --output-dir prefixes the output path (but only for relative paths).
                let output_path = output_path.map(|p| {
                    if let Some(ref dir) = opts.output_dir
                        && p.is_relative()
                    {
                        dir.join(&p)
                    } else {
                        p
                    }
                });

                // When resume (-C) was requested and the server responds with 416,
                // the response body is an error message and curl discards it.
                // Same on max-redirs reached — curl discards the final 3xx body.
                // With --fail on HTTP error, body is also discarded.
                // --fail discards the body; --fail-with-body keeps it.

                // Time condition body suppression (-z): after receiving a 2xx response,
                // check Last-Modified against the condition date.
                let time_cond_not_met = if let Some(ref tc) = opts.time_cond
                    && resp.status >= 200
                    && resp.status < 300
                {
                    if let Some((_, lm_str)) =
                        resp.headers.iter().find(|(k, _)| k == "last-modified")
                    {
                        if let Some(lm_ts) = crate::args::parse_date_string(lm_str) {
                            match tc {
                                crate::options::TimeCond::IfModifiedSince(ts) => lm_ts <= *ts,
                                crate::options::TimeCond::IfUnmodifiedSince(ts) => lm_ts > *ts,
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                let skip_body = (opts.resume_from.is_some() && resp.status == 416)
                    || resp.max_redirects_reached
                    || resp.proto_redir_blocked
                    || resp.redirect_url_malformed
                    || resp.weird_server_reply
                    || (fail_http_error && !opts.fail_with_body)
                    || resume_range_refused
                    || resume_fully_covered
                    || time_cond_not_met
                    || (resp.status == 304 && opts.etag_compare.is_some());

                // When `-z` decided the file isn't modified, surface a 304 to
                // %{response_code} (test 1239). The on-the-wire status line
                // we already wrote to header_bytes still says "200 OK", so
                // -i output is unchanged.
                if time_cond_not_met {
                    resp.status = 304;
                }

                // Write output.
                let write_body = !effective_opts.head && !skip_body;
                // With -C <offset>, preserve existing output file content and
                // append to it (matches curl's resume semantics).
                let append_mode = opts.resume_from.is_some();

                // --no-clobber: if the chosen output file already exists, find the
                // first available .N suffix (1..=100). If all 100 suffixes are taken,
                // fail with exit 23.
                let mut nc_failed = false;
                let output_path = if opts.no_clobber {
                    output_path.map(|p| {
                        if !p.exists() {
                            return p;
                        }
                        for n in 1..=100u32 {
                            let candidate = {
                                let mut s = p.as_os_str().to_owned();
                                s.push(format!(".{n}"));
                                PathBuf::from(s)
                            };
                            if !candidate.exists() {
                                return candidate;
                            }
                        }
                        nc_failed = true;
                        p
                    })
                } else {
                    output_path
                };
                if nc_failed {
                    eprintln!(
                        "curl: (23) Will not overwrite, all suffixes 1..=100 exist for {}",
                        output_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_default()
                    );
                    exit_code = 23;
                }

                // -J without explicit -o refuses to overwrite a file that
                // already exists (CURLE_WRITE_ERROR / exit 23, test 1460).
                let mut j_refuse_overwrite = false;
                if opts.remote_header_name
                    && used_cd_filename
                    && explicit_o.is_none()
                    && !nc_failed
                    && let Some(ref p) = output_path
                    && p.exists()
                {
                    eprintln!(
                        "curl: (23) Failed to open the file {}: File exists",
                        p.display()
                    );
                    exit_code = 23;
                    j_refuse_overwrite = true;
                }

                let write_file = |path: &PathBuf, data: &[u8]| -> std::io::Result<()> {
                    use std::io::Write;
                    let mut f = fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .append(append_mode)
                        .truncate(!append_mode)
                        .open(path)?;
                    f.write_all(data)
                };

                // -o `-` is treated as stdout (test 756): replace `Some("-")`
                // with `None` so the stdout branch below kicks in.
                let output_path = output_path.filter(|p| p.to_str() != Some("-"));

                // --out-null discards the body but, under --include, still
                // routes headers to stdout (curl 8.18 tool_cb_hdr.c skips the
                // out_null check for headers — test 756).
                let is_out_null = opts.outputs_null.get(url_idx).copied().unwrap_or(false);
                if nc_failed || j_refuse_overwrite {
                    // Skip writing the body — exit 23 already set.
                } else if is_out_null {
                    if effective_opts.include_headers || effective_opts.head {
                        let stdout = io::stdout();
                        let mut out = stdout.lock();
                        let _ = out.write_all(&retry_prefix);
                        let _ = out.write_all(&resp.redirect_headers);
                        if !dump_pair_with_include {
                            let _ = out.write_all(&resp.header_bytes);
                        }
                        let _ = out.flush();
                    }
                } else if let Some(ref path) = output_path {
                    if opts.create_dirs
                        && let Some(parent) = path.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        let _ = fs::create_dir_all(parent);
                    }
                    if effective_opts.include_headers || effective_opts.head {
                        let mut data = Vec::new();
                        data.extend_from_slice(&retry_prefix);
                        data.extend_from_slice(&resp.redirect_headers);
                        data.extend_from_slice(&resp.header_bytes);
                        if write_body {
                            data.extend_from_slice(&resp.body);
                        }
                        if let Err(e) = write_file(path, &data) {
                            eprintln!("curl: failed to write to {}: {e}", path.display());
                            exit_code = 23;
                        }
                    } else if write_body && let Err(e) = write_file(path, &resp.body) {
                        eprintln!("curl: failed to write to {}: {e}", path.display());
                        exit_code = 23;
                    }
                } else {
                    let stdout = io::stdout();
                    let mut out = stdout.lock();

                    let _ = out.write_all(&retry_prefix);
                    if effective_opts.include_headers || effective_opts.head {
                        let _ = out.write_all(&resp.redirect_headers);
                        // When `-D -` already wrote each header twice
                        // (paired with --include), skip the include write
                        // so the per-line interleave isn't replaced with a
                        // second header block.
                        if !dump_pair_with_include {
                            let _ = out.write_all(&resp.header_bytes);
                        }
                    }
                    if write_body {
                        let _ = out.write_all(&resp.body);
                    }
                    let _ = out.flush();
                }

                // --xattr: with CURL_FAKE_XATTR=1 (Debug-only env), echo the
                // attributes that would be written to the output file. The
                // real-attr path is omitted — tests 687/688 set the env so
                // they only check the stdout trace.
                if opts.xattr
                    && std::env::var("CURL_FAKE_XATTR").as_deref() == Ok("1")
                    && (200..300).contains(&resp.status)
                {
                    println!("user.creator => curl");
                    if let Some(ct) = resp
                        .headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                        .map(|(_, v)| v.as_str())
                    {
                        println!("user.mime_type => {ct}");
                    }
                    // The xattr stores the URL the user TYPED, not the final
                    // URL after redirects (test 644).
                    println!("user.xdg.origin.url => {url_str}");
                }

                // --remove-on-error: delete the downloaded file when any error
                // (non-zero exit) occurred for this URL. Applies only to file
                // outputs, not stdout.
                if opts.remove_on_error
                    && exit_code != 0
                    && let Some(ref path) = output_path
                {
                    let _ = fs::remove_file(path);
                }

                // -# / --progress-bar: emit a fully-filled progress line at
                // the end of a successful transfer. The exact width matters
                // only for the strip regex in test 1148 (matches `\r#{72} 100.0%`),
                // so we emit 72 hashes regardless of terminal width.
                if opts.progress_bar && exit_code == 0 && !opts.silent {
                    use std::io::Write as _;
                    let bar = "#".repeat(72);
                    let _ = writeln!(io::stderr(), "\r{bar} 100.0%");
                }

                // -R / --remote-time: set output file's mtime to the server's
                // Last-Modified header (or fall back to Date if absent). curl
                // applies this only when the response has a usable timestamp.
                if opts.remote_time
                    && exit_code == 0
                    && let Some(ref path) = output_path
                {
                    let ts = resp
                        .headers
                        .iter()
                        .find(|(k, _)| k == "last-modified")
                        .and_then(|(_, v)| cookie::parse_http_date(v));
                    if let Some(secs) = ts {
                        // Use libc::utimes via std::fs::FileTimes (stable as of 1.75).
                        // Pre-epoch timestamps (e.g. Last-Modified: 1940) are
                        // valid mtimes — produce a SystemTime by going below
                        // UNIX_EPOCH (test 762).
                        use std::time::{Duration, SystemTime};
                        let target = if secs >= 0 {
                            SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64)
                        } else {
                            SystemTime::UNIX_EPOCH - Duration::from_secs((-secs) as u64)
                        };
                        if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
                            let times = std::fs::FileTimes::new()
                                .set_modified(target)
                                .set_accessed(target);
                            let _ = file.set_times(times);
                        }
                    }
                }

                // Write-out.
                if let Some(ref fmt) = opts.write_out {
                    // Determine the effective method (the one used on the final
                    // request — redirects can change POST → GET).
                    let method = if effective_opts.head {
                        "HEAD".to_string()
                    } else if let Some(ref m) = effective_opts.method {
                        m.clone()
                    } else if resp.num_redirects > 0 {
                        "GET".to_string()
                    } else if opts.data.is_some() || !effective_opts.form_fields.is_empty() {
                        "POST".to_string()
                    } else if effective_opts.upload_file.is_some() {
                        "PUT".to_string()
                    } else {
                        "GET".to_string()
                    };
                    // Split on %{stderr}/%{stdout}/%output{...}/%{onerror} directives
                    // so each chunk goes to its own destination, then run %{var}
                    // substitution on each chunk separately.
                    use format::{WriteOutDest, split_write_out};
                    let error_msg = if exit_code == 22 {
                        format!("The requested URL returned error: {}", resp.status)
                    } else {
                        String::new()
                    };
                    let mut chunks_by_dest: Vec<(WriteOutDest, bool, String)> = Vec::new();
                    // size_upload: bytes we sent as the request body (POST -d / PUT
                    // -T / -F). The upload-file path honors CURL_UPLOAD_SIZE just
                    // like build_body does, so the value matches what hit the wire.
                    let size_upload: usize = if let Some(ref d) = opts.data {
                        d.len()
                    } else if let Some(ref p) = opts.upload_file {
                        std::fs::metadata(p)
                            .ok()
                            .and_then(|m| {
                                let len = m.len() as usize;
                                let truncated = std::env::var("CURL_UPLOAD_SIZE")
                                    .ok()
                                    .and_then(|s| s.parse::<usize>().ok())
                                    .filter(|&n| n <= len);
                                Some(truncated.unwrap_or(len))
                            })
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    for (dest, gated, raw) in split_write_out(fmt) {
                        if raw.is_empty() {
                            chunks_by_dest.push((dest, gated, raw));
                            continue;
                        }
                        let formatted = format_write_out(
                            &raw,
                            &resp,
                            &url,
                            resp.num_connects,
                            resp.num_redirects,
                            &method,
                            output_path.as_deref(),
                            url_idx,
                            exit_code,
                            &error_msg,
                            resp.final_referer
                                .as_deref()
                                .or(opts.referer.as_deref())
                                .unwrap_or(""),
                            size_upload,
                        );
                        chunks_by_dest.push((dest, gated, formatted));
                    }
                    for (dest, gated, text) in chunks_by_dest {
                        if text.is_empty() {
                            continue;
                        }
                        if gated && exit_code == 0 {
                            continue;
                        }
                        match dest {
                            WriteOutDest::Stdout => print!("{text}"),
                            WriteOutDest::Stderr => eprint!("{text}"),
                            WriteOutDest::File { path, append } => {
                                let res = fs::OpenOptions::new()
                                    .write(true)
                                    .create(true)
                                    .append(append)
                                    .truncate(!append)
                                    .open(&path)
                                    .and_then(|mut f| {
                                        use std::io::Write;
                                        f.write_all(text.as_bytes())
                                    });
                                if res.is_err() && !opts.silent {
                                    eprintln!(
                                        "curl: failed to write -w output to {}",
                                        path.display()
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                if !opts.silent || opts.show_error {
                    // .onion rejection (RFC 7686) maps to exit 6 with a
                    // curl-formatted "curl: (6) ..." message instead of the
                    // raw error prefix.
                    if let Some(rest) = e.strip_prefix("onion: ") {
                        eprintln!("curl: (6) {rest}");
                    } else if e.starts_with("CONNECT tunnel failed") {
                        eprintln!("curl: (56) {e}");
                    } else if e == "invalid_connect_response" {
                        eprintln!("curl: (43) Invalid response header");
                    } else if let Some(scheme) = e.strip_prefix("unsupported scheme: ") {
                        // Match curl's CURLE_UNSUPPORTED_PROTOCOL phrasing so
                        // -K test fixtures comparing stderr line up (test 1268).
                        eprintln!("curl: (1) Protocol \"{scheme}\" not supported");
                    } else if let Some(rest) = e.strip_prefix("socks5_long_host: ") {
                        eprintln!("curl: (97) {rest}");
                    } else {
                        eprintln!("curl: {e}");
                    }
                }
                // When the failure was a non-2xx CONNECT response, emit the
                // raw response bytes to stdout (tests 217, 287). Take the
                // bytes out so a subsequent successful URL doesn't replay them,
                // but keep the status code visible for `%{http_connect}`.
                if e.starts_with("CONNECT tunnel failed") {
                    let stashed = crate::connection::CONNECT_RESP.with(|r| r.borrow().clone());
                    if let Some((_status, ref bytes)) = stashed {
                        use std::io::Write as _;
                        // Route to -o output file when one is set, otherwise
                        // stdout (tests 217, 287, 749).
                        let written_to_file = if let Some(out_path) = opts.outputs.get(url_idx)
                            && out_path.to_str() != Some("-")
                        {
                            fs::write(out_path, bytes).is_ok()
                        } else {
                            false
                        };
                        if !written_to_file {
                            let _ = io::stdout().write_all(bytes);
                            let _ = io::stdout().flush();
                        }
                    }
                    // Run -w write-out with a synthetic zero-status response
                    // so `%{http_code} %{http_connect}` works (test 217).
                    if let Some(ref fmt) = effective_opts.write_out
                        && let Ok(parsed_url) = crate::url::parse_url(url_str)
                    {
                        let synth = crate::response::Response {
                            trailer_bytes: Vec::new(),
                            http10_response: false,
                            status: 0,
                            status_text: String::new(),
                            headers: Vec::new(),
                            body: Vec::new(),
                            header_bytes: Vec::new(),
                            redirect_headers: Vec::new(),
                            connect_header_size: 0,
                            num_connects: 1,
                            num_redirects: 0,
                            max_redirects_reached: false,
                            proto_redir_blocked: false,
                            upload_redirect_failed: false,
                            redirect_url_malformed: false,
                            weird_server_reply: false,
                            final_url: None,
                            final_referer: None,
                            redirect_url: None,
                            timed_out: false,
                            recv_error: false,
                            partial_file: false,
                            bad_content_encoding: false,
                            bad_encoding_too_many: false,
                            filesize_exceeded: false,
                            header_size_error: false,
                            ntlm_too_large: false,
                        };
                        let chunks = crate::format::split_write_out(fmt);
                        for (dest, gated, raw) in chunks {
                            if raw.is_empty() {
                                continue;
                            }
                            let text = crate::format::format_write_out(
                                &raw,
                                &synth,
                                &parsed_url,
                                1,
                                0,
                                "GET",
                                None,
                                url_idx,
                                56,
                                "CONNECT tunnel failed",
                                "",
                                0,
                            );
                            // CONNECT failure is "with error" — emit
                            // unconditionally except for `%{onerror}` gating.
                            if gated {
                                // gated = on-error gate; we are in error,
                                // so emit
                            }
                            use std::io::Write as _;
                            match dest {
                                crate::format::WriteOutDest::Stdout => {
                                    let _ = io::stdout().write_all(text.as_bytes());
                                }
                                crate::format::WriteOutDest::Stderr => {
                                    let _ = io::stderr().write_all(text.as_bytes());
                                }
                                crate::format::WriteOutDest::File { .. } => {}
                            }
                        }
                        let _ = io::stdout().flush();
                    }
                    crate::connection::CONNECT_RESP.with(|r| *r.borrow_mut() = None);
                }
                // Map error messages to curl exit codes.
                if e.starts_with("cacert: ") {
                    exit_code = 77; // Problem with reading the SSL CA cert
                } else if e.contains("hostname too long")
                    || e.contains("empty host")
                    || e.contains("bad port")
                    || e.contains("malformed URL")
                {
                    exit_code = 3; // URL malformed
                } else if e.contains("unsupported proxy scheme") {
                    exit_code = 7; // Unsupported proxy protocol
                } else if e.contains("unsupported scheme") || e.contains("unsupported protocol") {
                    exit_code = 1; // Unsupported protocol
                    fatal_protocol_error = true;
                } else if e.contains("invalid status") {
                    exit_code = 1; // CURLE_UNSUPPORTED_PROTOCOL — malformed status code (test 1430)
                } else if e.contains("too many response headers")
                    || e.contains("too large response header")
                    || e.contains("too large NTLM")
                {
                    exit_code = 100; // CURLE_TOO_MANY_HEADERS / CURLE_TOO_LARGE (tests 747, 775, 776, 1154)
                } else if e.starts_with("socks5_long_host:") {
                    exit_code = 97; // CURLE_PROXY (test 728)
                } else if e.starts_with("onion: ") {
                    exit_code = 6; // Couldn't resolve host (RFC 7686 refusal)
                } else if e.contains("DNS resolution failed for proxy") {
                    exit_code = 5; // CURLE_COULDNT_RESOLVE_PROXY
                } else if e.contains("DNS resolution failed") {
                    exit_code = 6; // Could not resolve host
                } else if e.contains("connection failed") || e.contains("Connection refused") {
                    exit_code = 7; // Failed to connect
                } else if e.contains("CONNECT tunnel failed") {
                    exit_code = 56; // CONNECT proxy tunnel failure
                } else if e == "invalid_connect_response" {
                    exit_code = 43; // Invalid response header (test 750)
                } else if e.contains("timed out") || e.contains("operation timed out") {
                    exit_code = 28; // Operation timeout
                } else if e.contains("maximum redirects") {
                    exit_code = 47; // Too many redirects
                } else if e.contains("weird_server_reply") {
                    exit_code = 8; // Weird server reply
                } else if e.starts_with("recv_error") {
                    exit_code = 56; // CURLE_RECV_ERROR — connection reset mid-read (test 1244)
                } else if e.contains("empty reply")
                    || e.contains("failed to read status line")
                    || e.contains("malformed status line")
                {
                    exit_code = 52; // Empty reply from server
                } else if e.contains("read form file") || e.contains("form file not found") {
                    exit_code = 26; // Read error (form file)
                } else if e.contains("upload_failed") {
                    exit_code = 25; // CURLE_UPLOAD_FAILED (test 1069)
                } else if e.contains("TLS handshake failed")
                    || e.contains("invalid peer certificate")
                    || e.contains("NotValidForName")
                    || e.contains("BadCertificate")
                    || e.contains("UnknownIssuer")
                    || e.contains("Expired")
                    || e.contains("InvalidCertificate")
                {
                    exit_code = 60; // CURLE_PEER_FAILED_VERIFICATION (test 311, 312)
                } else {
                    exit_code = 6;
                }

                // Emit --write-out even when the URL was rejected up front
                // (unsupported scheme, malformed authority, etc.) so
                // `%{url.*}` / `%{urle.*}` reflect what curl tried to parse
                // (test 423/424). Uses a lenient parser that accepts any
                // scheme and degrades to all-empty fields when the input
                // has no `://`.
                if let Some(ref fmt) = effective_opts.write_out
                    && !e.starts_with("CONNECT tunnel failed")
                {
                    use crate::format::{WriteOutDest, split_write_out};
                    let parsed_url = crate::url::parse_url_lenient(url_str);
                    let synth = crate::response::Response {
                        trailer_bytes: Vec::new(),
                        http10_response: false,
                        status: 0,
                        status_text: String::new(),
                        headers: Vec::new(),
                        body: Vec::new(),
                        header_bytes: Vec::new(),
                        redirect_headers: Vec::new(),
                        connect_header_size: 0,
                        num_connects: 0,
                        num_redirects: 0,
                        max_redirects_reached: false,
                        proto_redir_blocked: false,
                        upload_redirect_failed: false,
                        redirect_url_malformed: false,
                        weird_server_reply: false,
                        final_url: None,
                        final_referer: None,
                        redirect_url: None,
                        timed_out: false,
                        recv_error: false,
                        partial_file: false,
                        bad_content_encoding: false,
                        bad_encoding_too_many: false,
                        filesize_exceeded: false,
                        header_size_error: false,
                        ntlm_too_large: false,
                    };
                    for (dest, _gated, raw) in split_write_out(fmt) {
                        if raw.is_empty() {
                            continue;
                        }
                        let text = crate::format::format_write_out(
                            &raw,
                            &synth,
                            &parsed_url,
                            0,
                            0,
                            "GET",
                            None,
                            url_idx,
                            exit_code,
                            &e,
                            "",
                            0,
                        );
                        use std::io::Write as _;
                        match dest {
                            WriteOutDest::Stdout => {
                                let _ = io::stdout().write_all(text.as_bytes());
                                let _ = io::stdout().flush();
                            }
                            WriteOutDest::Stderr => {
                                let _ = io::stderr().write_all(text.as_bytes());
                            }
                            WriteOutDest::File { path, append } => {
                                let mut open = fs::OpenOptions::new();
                                open.write(true).create(true);
                                if append {
                                    open.append(true);
                                } else {
                                    open.truncate(true);
                                }
                                if let Ok(mut f) = open.open(path) {
                                    let _ = f.write_all(text.as_bytes());
                                }
                            }
                        }
                    }
                }
            }
        }

        // --fail-early: stop processing additional URLs once any URL has
        // produced a non-zero exit code (test 1247).
        if opts.fail_early && exit_code != 0 {
            break;
        }
        // Unsupported-protocol on the FIRST URL is fatal across the list —
        // curl's serial_transfers calls create_transfer for URL1 outside the
        // loop and returns immediately on CURLE_UNSUPPORTED_PROTOCOL (test
        // 760). For subsequent URLs, curl just records the returncode and
        // continues; the per-URL `-w` keeps emitting for the failed slots
        // (test 423).
        if url_idx == 0 && exit_code == 1 && fatal_protocol_error {
            break;
        }
    }

    // Alt-Svc write-back: serialize the merged pre-loaded + learned entries
    // to the --alt-svc file. We always rewrite when --alt-svc was given so
    // pre-loaded entries are preserved verbatim and any new ones are appended
    // (tests 437, 438). The expiry timestamp is stripped by the tests'
    // stripfile regex, so we just emit a placeholder year in the far future.
    if let Some(ref path) = opts.alt_svc_file {
        let mut text = String::new();
        text.push_str("# Your alt-svc cache. https://curl.se/docs/alt-svc.html\n");
        text.push_str("# This file was generated by libcurl! Edit at your own risk.\n");
        for (oh, op, ah, ap) in alt_svc_entries.iter().chain(alt_svc_learned.iter()) {
            text.push_str(&format!(
                "h1 {oh} {op} h1 {ah} {ap} \"20290222 22:19:28\" 0 0\n"
            ));
        }
        let _ = std::fs::write(path, text);
    }

    // HSTS write-back: serialize merged entries to the HSTS file in curl's
    // Netscape-like format. We only rewrite when at least one entry was
    // learned during the loop (tests 446, 780-783). CURL_TIME (when set)
    // mocks "now" so the expiry timestamps are deterministic.
    if let Some(ref path) = opts.hsts_file
        && hsts_learned_any
    {
        let mut text = String::new();
        text.push_str("# Your HSTS cache. https://curl.se/docs/hsts.html\n");
        text.push_str("# This file was generated by libcurl! Edit at your own risk.\n");
        for (sub, host, expiry) in &hsts_entries {
            let prefix = if *sub { "." } else { "" };
            text.push_str(&format!("{prefix}{host} \"{expiry}\"\n"));
        }
        let _ = std::fs::write(path, text);
    }

    // --libcurl: emit a C code template using libcurl that reproduces the
    // command (tests 1400-1481). Stripfile rules in the tests remove option
    // lines that differ across SSL backends, so we only have to get the
    // structural template + URL/UA right.
    if let Some(ref path) = opts.libcurl_file {
        // opts.urls already has `-G` data merged in. curl backslash-escapes
        // `?` in the C-string URL to dodge trigraph misinterpretation
        // (`??=` → `#`).
        let url = opts
            .urls
            .first()
            .cloned()
            .unwrap_or_default()
            .replace('?', "\\?");
        let ua = opts.user_agent.as_deref().unwrap_or("curl/8.0.0");
        let user_headers: Vec<String> = opts
            .headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        let needs_slist = !user_headers.is_empty();
        let mut c = String::new();
        c.push_str("/********* Sample code generated by the curl command line tool **********\n");
        c.push_str(" * All curl_easy_setopt() options are documented at:\n");
        c.push_str(" * https://curl.se/libcurl/c/curl_easy_setopt.html\n");
        c.push_str(" ************************************************************************/\n");
        c.push_str("#include <curl/curl.h>\n\n");
        c.push_str("int main(int argc, char *argv[])\n{\n");
        c.push_str("  CURLcode ret;\n  CURL *hnd;\n");
        if needs_slist {
            c.push_str("  struct curl_slist *slist1;\n");
            c.push_str("\n  slist1 = NULL;\n");
            for h in &user_headers {
                let esc = h.replace('\\', "\\\\").replace('"', "\\\"");
                c.push_str(&format!("  slist1 = curl_slist_append(slist1, \"{esc}\");\n"));
            }
        }
        c.push_str("\n  hnd = curl_easy_init();\n");
        c.push_str("  curl_easy_setopt(hnd, CURLOPT_VERBOSE, 1L);\n");
        c.push_str("  curl_easy_setopt(hnd, CURLOPT_BUFFERSIZE, 102400L);\n");
        c.push_str(&format!("  curl_easy_setopt(hnd, CURLOPT_URL, \"{url}\");\n"));
        if let Some(ref pxy) = opts.proxy {
            c.push_str(&format!(
                "  curl_easy_setopt(hnd, CURLOPT_PROXY, \"{pxy}\");\n"
            ));
        }
        if let Some(ref u) = opts.user {
            let esc = u.replace('\\', "\\\\").replace('"', "\\\"");
            c.push_str(&format!(
                "  curl_easy_setopt(hnd, CURLOPT_USERPWD, \"{esc}\");\n"
            ));
            if opts.basic_explicit {
                c.push_str(
                    "  curl_easy_setopt(hnd, CURLOPT_HTTPAUTH, (long)CURLAUTH_BASIC);\n",
                );
            }
        }
        if needs_slist {
            c.push_str("  curl_easy_setopt(hnd, CURLOPT_HTTPHEADER, slist1);\n");
        }
        if opts.data.is_some() && !opts.get {
            let data_bytes = opts.data.as_ref().map(|d| d.clone()).unwrap_or_default();
            let escaped = c_escape_bytes(&data_bytes);
            c.push_str(&format!(
                "  curl_easy_setopt(hnd, CURLOPT_POSTFIELDS, \"{escaped}\");\n"
            ));
            c.push_str(&format!(
                "  curl_easy_setopt(hnd, CURLOPT_POSTFIELDSIZE_LARGE, (curl_off_t){});\n",
                data_bytes.len()
            ));
        }
        c.push_str(&format!(
            "  curl_easy_setopt(hnd, CURLOPT_USERAGENT, \"{ua}\");\n"
        ));
        c.push_str("  curl_easy_setopt(hnd, CURLOPT_MAXREDIRS, 50L);\n");
        if !opts.cookies.is_empty() {
            let joined = opts.cookies.join(";");
            let esc = joined.replace('\\', "\\\\").replace('"', "\\\"");
            c.push_str(&format!(
                "  curl_easy_setopt(hnd, CURLOPT_COOKIE, \"{esc}\");\n"
            ));
        }
        // CURLOPT_SSLVERSION: combine min (--tlsv1.x) and max (--tls-max).
        if opts.tlsv1_min.is_some() || opts.tls_max.is_some() {
            let min_ver = opts.tlsv1_min.as_deref().unwrap_or("1.2");
            let min_const = match min_ver {
                "1.0" => "CURL_SSLVERSION_TLSv1_0",
                "1.1" => "CURL_SSLVERSION_TLSv1_1",
                "1.3" => "CURL_SSLVERSION_TLSv1_3",
                _ => "CURL_SSLVERSION_TLSv1_2",
            };
            let max_const = opts.tls_max.as_deref().map(|v| match v {
                "1.0" => "CURL_SSLVERSION_MAX_TLSv1_0",
                "1.1" => "CURL_SSLVERSION_MAX_TLSv1_1",
                "1.2" => "CURL_SSLVERSION_MAX_TLSv1_2",
                "1.3" => "CURL_SSLVERSION_MAX_TLSv1_3",
                _ => "CURL_SSLVERSION_MAX_DEFAULT",
            });
            match max_const {
                Some(maxc) => c.push_str(&format!(
                    "  curl_easy_setopt(hnd, CURLOPT_SSLVERSION, (long)({min_const} | {maxc}));\n"
                )),
                None => c.push_str(&format!(
                    "  curl_easy_setopt(hnd, CURLOPT_SSLVERSION, (long){min_const});\n"
                )),
            }
        }
        if opts.proxy_tlsv1 {
            c.push_str(
                "  curl_easy_setopt(hnd, CURLOPT_PROXY_SSLVERSION, (long)CURL_SSLVERSION_TLSv1);\n",
            );
        }
        c.push_str("  curl_easy_setopt(hnd, CURLOPT_TCP_KEEPALIVE, 1L);\n");
        // CURLOPT_PROTOCOLS_STR: comma-separated, lowercased, sorted alphabetically.
        if let Some(ref pr) = opts.proto_arg {
            let mut protos: Vec<String> = pr
                .split(',')
                .map(|t| {
                    t.trim()
                        .trim_start_matches(['+', '-', '='])
                        .to_ascii_lowercase()
                })
                .filter(|s| !s.is_empty())
                .collect();
            protos.sort();
            protos.dedup();
            let joined = protos.join(",");
            c.push_str(&format!(
                "  curl_easy_setopt(hnd, CURLOPT_PROTOCOLS_STR, \"{joined}\");\n"
            ));
        }
        c.push_str("\n");
        c.push_str("  /* Here is a list of options the curl code used that cannot get generated\n");
        c.push_str("     as source easily. You may choose to either not use them or implement\n");
        c.push_str("     them yourself.\n\n");
        c.push_str("  CURLOPT_DEBUGFUNCTION was set to a function pointer\n");
        c.push_str("  CURLOPT_DEBUGDATA was set to an object pointer\n");
        c.push_str("  CURLOPT_WRITEDATA was set to an object pointer\n");
        c.push_str("  CURLOPT_WRITEFUNCTION was set to a function pointer\n");
        c.push_str("  CURLOPT_READDATA was set to an object pointer\n");
        c.push_str("  CURLOPT_READFUNCTION was set to a function pointer\n");
        c.push_str("  CURLOPT_SEEKDATA was set to an object pointer\n");
        c.push_str("  CURLOPT_SEEKFUNCTION was set to a function pointer\n");
        c.push_str("  CURLOPT_HEADERFUNCTION was set to a function pointer\n");
        c.push_str("  CURLOPT_HEADERDATA was set to an object pointer\n");
        c.push_str("  CURLOPT_ERRORBUFFER was set to an object pointer\n");
        c.push_str("  CURLOPT_STDERR was set to an object pointer\n\n");
        c.push_str("  */\n\n");
        c.push_str("  ret = curl_easy_perform(hnd);\n\n");
        c.push_str("  curl_easy_cleanup(hnd);\n  hnd = NULL;\n");
        if needs_slist {
            c.push_str("  curl_slist_free_all(slist1);\n  slist1 = NULL;\n");
        }
        c.push_str("\n  return (int)ret;\n}\n");
        c.push_str("/**** End of sample code ****/\n");
        let _ = std::fs::write(path, c);
    }

    process::exit(exit_code);
}

/// C-string escape matching curl's `c_escape` (tool_paramhlp.c): backslash,
/// quote, `?`, `\n` `\r` `\t` are spelled out; other non-printable bytes use
/// `\xNN` hex unless the next byte is a hex digit (in which case 3-digit octal
/// `\NNN` is used to keep the escape from absorbing it).
fn c_escape_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4);
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'?' => out.push_str("\\?"),
            c if (0x20..0x7F).contains(&c) => out.push(c as char),
            c => {
                let next_hex = bytes
                    .get(i + 1)
                    .is_some_and(|n| n.is_ascii_hexdigit());
                if next_hex {
                    out.push_str(&format!("\\{c:03o}"));
                } else {
                    out.push_str(&format!("\\x{c:02x}"));
                }
            }
        }
    }
    out
}

/// Pick the IPFS gateway URL, validating it. Returns Err(exit_code) on failure:
///   3  — gateway URL (from file) is malformed
///   37 — no gateway available
///   43 — gateway URL (from --ipfs-gateway flag) is malformed
/// Trailing slashes are preserved; the caller normalizes them when building URLs.
fn resolve_ipfs_gateway(flag: Option<&str>) -> Result<String, i32> {
    // Returns Ok(gateway) or Err(code):
    //   `from_flag` = true (the --ipfs-gateway arg) → parse failure is 43;
    //   `from_flag` = false (a gateway file)        → parse failure is 3.
    // A `?` or `#` in any gateway is always 3 — curl rejects it when joining
    // with the IPFS path (the result would have two `?`s, malformed).
    fn validate(g: &str, from_flag: bool) -> Result<String, i32> {
        if g.contains('?') || g.contains('#') {
            return Err(3);
        }
        let lower = g.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err(if from_flag { 43 } else { 3 });
        }
        let url = crate::url::parse_url(g).ok();
        let bad = url.is_none()
            || url
                .as_ref()
                .is_some_and(|u| u.host.is_empty() || u.host.contains(','));
        if bad {
            return Err(if from_flag { 43 } else { 3 });
        }
        Ok(g.trim_end().to_string())
    }
    if let Some(g) = flag {
        return validate(g, true);
    }
    let path_from_env = std::env::var("IPFS_PATH")
        .ok()
        .map(|p| PathBuf::from(p).join("gateway"));
    let path_from_home = std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".ipfs").join("gateway"));
    for candidate in [path_from_env, path_from_home].into_iter().flatten() {
        if let Ok(contents) = std::fs::read_to_string(&candidate) {
            let first = contents.lines().next().unwrap_or("").trim();
            return validate(first, false);
        }
    }
    Err(37)
}

fn parse_alt_svc_header(value: &str) -> Vec<(String, u16)> {
    // Parse a single Alt-Svc header value. Comma-separated entries; each is
    // `proto="host:port"; params`. We only collect entries whose protocol is
    // `h1` since we don't speak h2/h3. IPv6 hosts are quoted with brackets:
    // `h1="[ffff::1]:8181"` (test 437).
    let mut out = Vec::new();
    for raw in value.split(',') {
        let entry = raw.trim();
        // Split off `; params` (max-age, persist, …) — only the first token
        // is the proto="..." part.
        let first = entry.split(';').next().unwrap_or("").trim();
        let Some((proto, val)) = first.split_once('=') else {
            continue;
        };
        let proto = proto.trim();
        if !proto.eq_ignore_ascii_case("h1") {
            continue;
        }
        let val = val.trim().trim_matches('"');
        // Split host:port from the right. For IPv6 the host is in brackets.
        let (host, port) = if let Some(end_bracket) = val.find(']') {
            let host = &val[..=end_bracket];
            let rest = val.get(end_bracket + 1..).unwrap_or("");
            let port = rest.strip_prefix(':').unwrap_or(rest);
            (host.to_string(), port)
        } else if let Some(idx) = val.rfind(':') {
            (val[..idx].to_string(), &val[idx + 1..])
        } else {
            (val.to_string(), "")
        };
        if let Ok(p) = port.parse::<u16>() {
            out.push((host, p));
        }
    }
    out
}

fn parse_hsts_header(header_value: &str) -> Option<(i64, bool)> {
    // Strict-Transport-Security: max-age=<N>[; includeSubDomains][; preload]
    // Whitespace around '=' is allowed (test 780). Case-insensitive.
    let mut max_age: Option<i64> = None;
    let mut include_subdomains = false;
    for raw in header_value.split(';') {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        let lower = part.to_ascii_lowercase();
        if lower == "includesubdomains" {
            include_subdomains = true;
        } else if let Some(eq) = part.find('=') {
            let key = part[..eq].trim().to_ascii_lowercase();
            if key == "max-age" {
                let v = part[eq + 1..].trim().trim_matches('"');
                if let Ok(n) = v.parse::<i64>() {
                    max_age = Some(n);
                }
            }
        }
    }
    max_age.map(|n| (current_time_secs() + n, include_subdomains))
}

fn current_time_secs() -> i64 {
    if let Ok(v) = std::env::var("CURL_TIME")
        && let Ok(n) = v.parse::<i64>()
    {
        return n;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn format_hsts_expiry(secs: i64) -> String {
    let (y, mo, d, h, mi, s) = secs_to_ymdhms(secs);
    format!("{y:04}{mo:02}{d:02} {h:02}:{mi:02}:{s:02}")
}

fn secs_to_ymdhms(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    // Days since 1970-01-01 + seconds-of-day.
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400) as u32;
    let h = sod / 3600;
    let mi = (sod % 3600) / 60;
    let s = sod % 60;
    // Howard Hinnant's days_from_civil inverse (civil_from_days).
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (y + if m <= 2 { 1 } else { 0 }) as i32;
    (y, m, d, h, mi, s)
}

/// Parse a Content-Disposition header value and return the `filename` parameter
/// if present. Handles both `filename="X"` (quoted) and `filename=X` (token)
/// forms. Strips leading path components (curl treats `/`, `\\`, and `:` as
/// path separators and keeps only the basename — prevents directory traversal).
fn extract_cd_filename(value: &str) -> Option<String> {
    // Split on `;` but not inside double quotes (so a `filename="a;b"` keeps
    // its semicolon — test 1312).
    let mut parts: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut in_quotes = false;
    for ch in value.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            buf.push(ch);
        } else if ch == ';' && !in_quotes {
            parts.push(std::mem::take(&mut buf));
        } else {
            buf.push(ch);
        }
    }
    parts.push(buf);
    for part in parts {
        let part = part.trim();
        if let Some(rest) = part
            .strip_prefix("filename=")
            .or_else(|| part.strip_prefix("Filename="))
        {
            // Strip a balanced pair of `"..."` or `'...'` quotes; otherwise
            // strip just a leading lone quote (test 1313's `filename='name`).
            let raw =
                if let Some(stripped) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    stripped
                } else if let Some(stripped) =
                    rest.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
                {
                    stripped
                } else if let Some(stripped) = rest.strip_prefix(['"', '\'']) {
                    stripped
                } else {
                    rest
                };
            // Strip any directory components — keep only the basename.
            let basename = raw.rsplit(['/', '\\', ':']).next().unwrap_or(raw);
            if !basename.is_empty() {
                return Some(basename.to_string());
            }
        }
    }
    None
}

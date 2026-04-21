mod args;
mod connection;
mod cookie;
mod format;
mod options;
mod request;
mod response;
mod tls;
mod url;

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
                    opts.proxy_user = Some(userinfo.to_string());
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

    // URL globbing: expand {a,b,c} and [1-10] style patterns in each URL
    // into separate URLs. -g/--globoff disables this.
    // Track glob values for each URL so #N in -o refers to the Nth glob value.
    let url_glob_values: Vec<Vec<String>>;
    if !opts.globoff {
        let mut all_expanded = Vec::new();
        for u in &opts.urls {
            match expand_glob(u) {
                Ok(expanded) => all_expanded.extend(expanded),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(3);
                }
            }
        }
        opts.urls = all_expanded.iter().map(|(url, _)| url.clone()).collect();
        url_glob_values = all_expanded.into_iter().map(|(_, vals)| vals).collect();
    } else {
        url_glob_values = opts.urls.iter().map(|_| Vec::new()).collect();
    }

    if opts.outputs.len() > opts.urls.len() && !opts.silent {
        eprintln!("Warning: Got more output options than URLs");
    }

    // Pre-flight: verify --etag-save path is writable. curl exits 26 (read/write
    // error) if the etag file can't be created, before attempting any transfer.
    if let Some(ref etag_path) = opts.etag_save
        && let Err(e) = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(etag_path)
    {
        eprintln!("curl: (26) Failed to open {}: {}", etag_path.display(), e);
        process::exit(26);
    }

    // -C - (auto-resume):
    //   - GET: use the existing output file's size as the resume offset.
    //   - PUT (-T): curl can't know server-side size, so it always starts from
    //     offset 0 (sends the full upload with Content-Range: bytes 0-N-1/N).
    if opts.resume_from.as_deref() == Some("-") {
        if opts.upload_file.is_some() {
            opts.resume_from = Some("0".to_string());
        } else if let Some(path) = opts.outputs.first()
            && let Ok(meta) = fs::metadata(path)
        {
            opts.resume_from = Some(meta.len().to_string());
        }
    }

    // Pre-validate --dump-header destination. "-" means stdout, "%" means stderr;
    // otherwise try to open as a file (curl fails early if the path is unusable).
    if let Some(ref dump_path) = opts.dump_header
        && dump_path.to_str() != Some("-")
        && dump_path.to_str() != Some("%")
        && fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(dump_path)
            .is_err()
    {
        eprintln!("curl: (23) Failed to open/create {}", dump_path.display());
        process::exit(23);
    }

    // -G / --get: append -d/--data-* content as a query string, and clear the
    // request body so these become GET (or HEAD with -I) requests.
    if opts.get && opts.data.is_some() {
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

    for (url_idx, url_str) in opts.urls.iter().enumerate() {
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
        let result = if opts.retry > 0 {
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
                match perform(url_str, &opts) {
                    Ok(r) => {
                        // Retry on 5xx and 429 (rate-limited).
                        if (r.status >= 500 || r.status == 429) && attempt < opts.retry {
                            last_err = format!("HTTP {}", r.status);
                            if opts.include_headers {
                                retry_prefix.extend_from_slice(&r.header_bytes);
                            }
                            retry_prefix.extend_from_slice(&r.body);
                            // Honor Retry-After header (seconds) for the delay.
                            next_delay_secs = r
                                .headers
                                .iter()
                                .find(|(k, _)| k.eq_ignore_ascii_case("retry-after"))
                                .and_then(|(_, v)| v.trim().parse::<u64>().ok())
                                .unwrap_or((attempt as u64 + 1) * 2);
                            resp = Some(r);
                            continue;
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
            perform(url_str, &opts)
        };

        match result {
            Ok(resp) => {
                // After redirects, the "effective" URL is the last URL we fetched;
                // parse_url(url_str) returns the initial one. Reconstruct the last
                // URL from the ParsedUrl we have on resp (via final-url tracking).
                let url = resp
                    .final_url
                    .as_deref()
                    .and_then(|u| parse_url(u).ok())
                    .unwrap_or_else(|| parse_url(url_str).unwrap());

                // Resume (-C) with 416 means the file was already fully downloaded —
                // treat as success even with --fail.
                let resume_fully_downloaded = opts.resume_from.is_some() && resp.status == 416;
                // Resume (-C) on a GET with 200 (not 206) AND no Content-Range means
                // the server didn't honor our Range — curl reports error 33
                // (CURLE_RANGE_ERROR) and skips the body. Only for downloads.
                // But: if the response's Content-Length matches the requested
                // resume offset, we already have everything — treat as success.
                let has_content_range = resp.headers.iter().any(|(k, _)| k == "content-range");
                let resume_offset: Option<u64> =
                    opts.resume_from.as_deref().and_then(|s| s.parse().ok());
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
                let resume_range_refused = opts.resume_from.is_some()
                    && resp.status == 200
                    && !has_content_range
                    && !resume_fully_covered
                    && opts.upload_file.is_none();
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
                if let Some(ref dump_path) = opts.dump_header {
                    match dump_path.to_str() {
                        Some("-") => {
                            let _ = io::stdout().write_all(&resp.header_bytes);
                        }
                        Some("%") => {
                            let _ = io::stderr().write_all(&resp.header_bytes);
                        }
                        _ => {
                            if fs::write(dump_path, &resp.header_bytes).is_err() {
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
                            opts.memory_cookies.push(line);
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
                let output_path = if let Some(name) = cd_filename {
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
                    || resp.weird_server_reply
                    || (fail_http_error && !opts.fail_with_body)
                    || resume_range_refused
                    || resume_fully_covered
                    || time_cond_not_met;

                // Write output.
                let write_body = !opts.head && !skip_body;
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

                if nc_failed {
                    // Skip writing the body — exit 23 already set.
                } else if let Some(ref path) = output_path {
                    if opts.include_headers || opts.head {
                        let mut data = Vec::new();
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
                    if opts.include_headers || opts.head {
                        let _ = out.write_all(&resp.redirect_headers);
                        let _ = out.write_all(&resp.header_bytes);
                    }
                    if write_body {
                        let _ = out.write_all(&resp.body);
                    }
                    let _ = out.flush();
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

                // Write-out.
                if let Some(ref fmt) = opts.write_out {
                    // Determine the effective method (the one used on the final
                    // request — redirects can change POST → GET).
                    let method = if opts.head {
                        "HEAD".to_string()
                    } else if let Some(ref m) = opts.method {
                        m.clone()
                    } else if resp.num_redirects > 0 {
                        "GET".to_string()
                    } else if opts.data.is_some() || !opts.form_fields.is_empty() {
                        "POST".to_string()
                    } else if opts.upload_file.is_some() {
                        "PUT".to_string()
                    } else {
                        "GET".to_string()
                    };
                    // Split on %{stderr}/%{stdout}/%output{...} directives so each
                    // chunk goes to its own destination, then run %{var} substitution
                    // on each chunk separately.
                    use format::{WriteOutDest, split_write_out};
                    let mut chunks_by_dest: Vec<(WriteOutDest, String)> = Vec::new();
                    for (dest, raw) in split_write_out(fmt) {
                        if raw.is_empty() {
                            chunks_by_dest.push((dest, raw));
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
                        );
                        chunks_by_dest.push((dest, formatted));
                    }
                    for (dest, text) in chunks_by_dest {
                        if text.is_empty() {
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
                    } else {
                        eprintln!("curl: {e}");
                    }
                }
                // Map error messages to curl exit codes.
                if e.contains("unsupported proxy scheme") {
                    exit_code = 7; // Unsupported proxy protocol
                } else if e.contains("unsupported scheme") || e.contains("unsupported protocol") {
                    exit_code = 1; // Unsupported protocol
                } else if e.starts_with("onion: ") {
                    exit_code = 6; // Couldn't resolve host (RFC 7686 refusal)
                } else if e.contains("DNS resolution failed") {
                    exit_code = 6; // Could not resolve host
                } else if e.contains("connection failed") || e.contains("Connection refused") {
                    exit_code = 7; // Failed to connect
                } else if e.contains("CONNECT tunnel failed") {
                    exit_code = 56; // CONNECT proxy tunnel failure
                } else if e.contains("timed out") || e.contains("operation timed out") {
                    exit_code = 28; // Operation timeout
                } else if e.contains("maximum redirects") {
                    exit_code = 47; // Too many redirects
                } else if e.contains("weird_server_reply") {
                    exit_code = 8; // Weird server reply
                } else if e.contains("empty reply")
                    || e.contains("failed to read status line")
                    || e.contains("malformed status line")
                {
                    exit_code = 52; // Empty reply from server
                } else if e.contains("read form file") || e.contains("form file not found") {
                    exit_code = 26; // Read error (form file)
                } else {
                    exit_code = 6;
                }
            }
        }
    }

    process::exit(exit_code);
}

/// Parse a Content-Disposition header value and return the `filename` parameter
/// if present. Handles both `filename="X"` (quoted) and `filename=X` (token)
/// forms. Strips leading path components (curl treats `/`, `\\`, and `:` as
/// path separators and keeps only the basename — prevents directory traversal).
fn extract_cd_filename(value: &str) -> Option<String> {
    for part in value.split(';') {
        let part = part.trim();
        if let Some(rest) = part
            .strip_prefix("filename=")
            .or_else(|| part.strip_prefix("Filename="))
        {
            let raw = rest
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(rest);
            // Strip any directory components — keep only the basename.
            let basename = raw.rsplit(['/', '\\', ':']).next().unwrap_or(raw);
            if !basename.is_empty() {
                return Some(basename.to_string());
            }
        }
    }
    None
}

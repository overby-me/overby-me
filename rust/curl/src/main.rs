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
use url::parse_url;

fn main() {
    let opts = parse_args();
    let mut exit_code = 0;

    for url_str in &opts.urls {
        let result = if opts.retry > 0 {
            let mut last_err = String::new();
            let mut resp = None;
            for attempt in 0..=opts.retry {
                if attempt > 0 {
                    if !opts.silent {
                        eprintln!(
                            "Warning: Transient problem. Will retry in {} seconds. ({attempt}/{} retries)",
                            attempt * 2,
                            opts.retry,
                        );
                    }
                    std::thread::sleep(Duration::from_secs((attempt * 2) as u64));
                }
                match perform(url_str, &opts) {
                    Ok(r) => {
                        // Retry on 5xx.
                        if r.status >= 500 && attempt < opts.retry {
                            last_err = format!("HTTP {}", r.status);
                            continue;
                        }
                        resp = Some(r);
                        break;
                    }
                    Err(e) => {
                        last_err = e;
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
                let url = parse_url(url_str).unwrap();

                if opts.fail && resp.status >= 400 {
                    if opts.show_error || !opts.silent {
                        eprintln!(
                            "curl: (22) The requested URL returned error: {} {}",
                            resp.status, resp.status_text
                        );
                    }
                    exit_code = 22;
                    continue;
                }

                // Dump headers to file.
                if let Some(ref dump_path) = opts.dump_header {
                    let _ = fs::write(dump_path, &resp.header_bytes);
                }

                // Save cookie jar.
                if let Some(ref jar_path) = opts.cookie_jar {
                    save_cookie_jar(jar_path, &url, &resp.headers);
                }

                // Determine output destination.
                // -o - means stdout (not a file called "-")
                let output_path = if opts.remote_name {
                    // Derive filename from URL path.
                    let name = url
                        .path
                        .rsplit('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("index.html");
                    Some(PathBuf::from(name))
                } else {
                    opts.output.clone().filter(|p| p.to_str() != Some("-"))
                };

                // Write output.
                if let Some(ref path) = output_path {
                    if opts.include_headers || opts.head {
                        let mut data = Vec::new();
                        // Include intermediate redirect headers
                        data.extend_from_slice(&resp.redirect_headers);
                        data.extend_from_slice(&resp.header_bytes);
                        if !opts.head {
                            data.extend_from_slice(&resp.body);
                        }
                        if let Err(e) = fs::write(path, &data) {
                            eprintln!("curl: failed to write to {}: {e}", path.display());
                            exit_code = 23;
                        }
                    } else if let Err(e) = fs::write(path, &resp.body) {
                        eprintln!("curl: failed to write to {}: {e}", path.display());
                        exit_code = 23;
                    }
                } else {
                    let stdout = io::stdout();
                    let mut out = stdout.lock();

                    if opts.include_headers || opts.head {
                        // Include intermediate redirect headers
                        let _ = out.write_all(&resp.redirect_headers);
                        let _ = out.write_all(&resp.header_bytes);
                    }
                    if !opts.head {
                        let _ = out.write_all(&resp.body);
                    }
                    let _ = out.flush();
                }

                // Write-out.
                if let Some(ref fmt) = opts.write_out {
                    let formatted = format_write_out(fmt, &resp, &url);
                    print!("{formatted}");
                }
            }
            Err(e) => {
                if !opts.silent || opts.show_error {
                    eprintln!("curl: {e}");
                }
                // Map error messages to curl exit codes.
                if e.contains("unsupported scheme") || e.contains("unsupported protocol") {
                    exit_code = 1; // Unsupported protocol
                } else if e.contains("DNS resolution failed") {
                    exit_code = 6; // Could not resolve host
                } else if e.contains("connection failed") || e.contains("Connection refused") {
                    exit_code = 7; // Failed to connect
                } else if e.contains("timed out") || e.contains("operation timed out") {
                    exit_code = 28; // Operation timeout
                } else if e.contains("maximum redirects") {
                    exit_code = 47; // Too many redirects
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

use std::io::{self, Read};
use std::path::PathBuf;
use std::time::Duration;
use std::{env, fs, process};

use crate::format::urlencode_field;
use crate::options::{FormField, Options};

/// If an argument starts with a UTF-8 multibyte lead byte (0xC0-0xFF), emit
/// curl's smart-quote warning. Used for both CLI args and config-file args.
fn check_unicode_warning(arg: &str) {
    let b = arg.as_bytes();
    if !b.is_empty() && (b[0] & 0xc0) == 0xc0 {
        warn_wrapped(&format!(
            "The argument '{arg}' starts with a Unicode character. Maybe ASCII was intended?"
        ));
    }
}

/// Read a curl config file and return the list of argument tokens it produces.
/// Supports `-K -` to read from stdin. Each non-blank, non-comment line yields
/// one option (with `--` prefix if missing) and optionally one value. Values
/// may be double-quoted with `\\` and `\"` escapes; unquoted values run to end
/// of line (trimmed).
fn read_config_file(path: &str) -> Vec<String> {
    let content = if path == "-" {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s).unwrap_or(0);
        s
    } else {
        fs::read_to_string(path).unwrap_or_default()
    };

    let mut out = Vec::new();
    for raw in content.split('\n') {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((opt, val)) = parse_config_line(line) {
            out.push(opt);
            if let Some(val) = val {
                out.push(val);
            }
        }
    }
    out
}

/// Parse a single config-file line into (option, optional value).
fn parse_config_line(line: &str) -> Option<(String, Option<String>)> {
    let mut chars = line.chars().peekable();

    // Optional leading dashes on the option name.
    let mut dashes = 0;
    while chars.peek() == Some(&'-') && dashes < 2 {
        chars.next();
        dashes += 1;
    }

    // Option name: letters, digits, dashes, underscores.
    let mut opt = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            opt.push(c);
            chars.next();
        } else {
            break;
        }
    }
    if opt.is_empty() {
        return None;
    }

    // Long-option default when no dashes given (curl treats bare names as long).
    let prefix = if dashes > 0 {
        "-".repeat(dashes)
    } else {
        "--".to_string()
    };
    let opt_str = format!("{prefix}{opt}");

    // Skip separator(s): space, tab, `=`, `:`.
    while let Some(&c) = chars.peek() {
        if c == ' ' || c == '\t' || c == '=' || c == ':' {
            chars.next();
        } else {
            break;
        }
    }

    if chars.peek().is_none() {
        return Some((opt_str, None));
    }

    // Value: optionally double-quoted, with \\ and \" escapes.
    let mut val = String::new();
    if chars.peek() == Some(&'"') {
        chars.next();
        while let Some(c) = chars.next() {
            if c == '"' {
                break;
            } else if c == '\\' {
                // Interpret C-style escapes inside quoted values: \t, \n, \r
                // become their literal control characters; all others (\\, \",
                // or unknown) are passed through as the following char.
                if let Some(next) = chars.next() {
                    match next {
                        't' => val.push('\t'),
                        'n' => val.push('\n'),
                        'r' => val.push('\r'),
                        other => val.push(other),
                    }
                }
            } else {
                val.push(c);
            }
        }
    } else {
        for c in chars {
            val.push(c);
        }
        val = val.trim().to_string();
    }

    Some((opt_str, Some(val)))
}

/// Emit a warning to stderr with curl's line-wrapping: each physical line is
/// at most 79 bytes including the "Warning: " prefix. Word-wraps on spaces,
/// preserving the trailing space at the wrap point (matching curl's voutf).
fn warn_wrapped(msg: &str) {
    const PREFIX: &str = "Warning: ";
    const LINE_WIDTH: usize = 79;
    let content_width = LINE_WIDTH - PREFIX.len();

    let mut line = String::new();
    for word in msg.split(' ') {
        if line.is_empty() {
            line.push_str(word);
        } else if line.len() + 1 + word.len() <= content_width {
            line.push(' ');
            line.push_str(word);
        } else {
            eprintln!("{PREFIX}{line} ");
            line.clear();
            line.push_str(word);
        }
    }
    if !line.is_empty() {
        eprintln!("{PREFIX}{line}");
    }
}

pub(crate) fn parse_args() -> Options {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let mut opts = Options::default();

    if args.is_empty() {
        print_usage();
        process::exit(0);
    }

    // Warn when an argument starts with a UTF-8 multibyte lead (0xC0-0xFF).
    // Common user mistake: shell smart quotes (U+2018, U+201C, …) that look
    // like ASCII quotes but aren't. Config-file args are checked when loaded.
    for arg in &args {
        check_unicode_warning(arg);
    }

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                process::exit(0);
            }
            "-V" | "--version" => {
                println!("curl 8.0.0 (rust-curl) libcurl/8.0.0 rustls/0.23");
                println!("Protocols: http https");
                println!("Features: HTTPS SSL libz");
                process::exit(0);
            }
            "-X" | "--request" => {
                i += 1;
                opts.method = Some(next_arg(&args, i, "-X"));
            }
            "-H" | "--header" => {
                i += 1;
                let h = next_arg(&args, i, "-H");
                parse_header_arg(&mut opts, &h);
            }
            "-d" | "--data" | "--data-ascii" => {
                i += 1;
                let val = next_arg(&args, i, "-d");
                append_data(&mut opts, &val, false);
            }
            "--json" => {
                i += 1;
                let val = next_arg(&args, i, "--json");
                // Like -d: @filename reads from file, @- from stdin, otherwise
                // the value is used as-is. (Not raw — @ is significant.)
                append_data(&mut opts, &val, false);
                opts.json = true;
            }
            "--data-raw" => {
                i += 1;
                let val = next_arg(&args, i, "--data-raw");
                opts.data_raw = true;
                append_data(&mut opts, &val, true);
            }
            "--data-binary" => {
                i += 1;
                let val = next_arg(&args, i, "--data-binary");
                append_data_binary(&mut opts, &val);
            }
            "--data-urlencode" => {
                i += 1;
                let val = next_arg(&args, i, "--data-urlencode");
                let encoded = urlencode_field(&val);
                append_data(&mut opts, &encoded, true);
            }
            "-F" | "--form" => {
                i += 1;
                let val = next_arg(&args, i, "-F");
                parse_form_field(&mut opts, &val);
            }
            "-o" | "--output" => {
                i += 1;
                opts.outputs.push(PathBuf::from(next_arg(&args, i, "-o")));
            }
            "-O" | "--remote-name" => {
                opts.remote_name = true;
            }
            "-J" | "--remote-header-name" => {
                opts.remote_header_name = true;
            }
            "--output-dir" => {
                i += 1;
                opts.output_dir = Some(PathBuf::from(next_arg(&args, i, "--output-dir")));
            }
            "-L" | "--location" => {
                opts.location = true;
            }
            "--location-trusted" => {
                opts.location = true;
                opts.location_trusted = true;
            }
            "--tr-encoding" => {
                opts.tr_encoding = true;
            }
            "--no-tr-encoding" => {
                opts.tr_encoding = false;
            }
            "--max-redirs" => {
                i += 1;
                opts.max_redirs = next_arg(&args, i, "--max-redirs").parse().unwrap_or(50);
            }
            "--max-filesize" => {
                i += 1;
                let val = next_arg(&args, i, "--max-filesize");
                opts.max_filesize = val.parse().ok();
                opts.max_filesize_str = Some(val);
            }
            "-v" | "--verbose" => {
                opts.verbose = true;
            }
            "-s" | "--silent" => {
                opts.silent = true;
            }
            "-S" | "--show-error" => {
                opts.show_error = true;
            }
            "-f" | "--fail" => {
                if opts.fail_with_body {
                    eprintln!("Warning: --fail deselects --fail-with-body here");
                    opts.fail_with_body = false;
                }
                opts.fail = true;
            }
            "--fail-with-body" => {
                if opts.fail {
                    eprintln!("Warning: --fail-with-body deselects --fail here");
                    opts.fail = false;
                }
                opts.fail_with_body = true;
            }
            "-i" | "--include" => {
                opts.include_headers = true;
            }
            "-I" | "--head" => {
                opts.head = true;
            }
            "-A" | "--user-agent" => {
                i += 1;
                opts.user_agent = Some(next_arg(&args, i, "-A"));
            }
            "-e" | "--referer" => {
                i += 1;
                opts.referer = Some(next_arg(&args, i, "-e"));
            }
            "-b" | "--cookie" => {
                i += 1;
                let val = next_arg(&args, i, "-b");
                opts.cookies.push(val);
                opts.cookie_engine = true;
            }
            "-c" | "--cookie-jar" => {
                i += 1;
                opts.cookie_jar = Some(PathBuf::from(next_arg(&args, i, "-c")));
            }
            "-u" | "--user" => {
                i += 1;
                opts.user = Some(next_arg(&args, i, "-u"));
            }
            "--connect-timeout" => {
                i += 1;
                let secs: f64 = next_arg(&args, i, "--connect-timeout")
                    .parse()
                    .unwrap_or(0.0);
                opts.connect_timeout = Some(Duration::from_secs_f64(secs));
            }
            "-m" | "--max-time" => {
                i += 1;
                let secs: f64 = next_arg(&args, i, "--max-time").parse().unwrap_or(0.0);
                opts.max_time = Some(Duration::from_secs_f64(secs));
            }
            "-k" | "--insecure" => {
                opts.insecure = true;
            }
            "--compressed" => {
                opts.compressed = true;
            }
            "-D" | "--dump-header" => {
                i += 1;
                opts.dump_header = Some(PathBuf::from(next_arg(&args, i, "-D")));
            }
            "-w" | "--write-out" => {
                i += 1;
                opts.write_out = Some(next_arg(&args, i, "-w"));
            }
            "--retry" => {
                i += 1;
                opts.retry = next_arg(&args, i, "--retry").parse().unwrap_or(0);
            }
            "--retry-max-time" => {
                i += 1;
                let val = next_arg(&args, i, "--retry-max-time");
                match val.parse::<u32>() {
                    Ok(n) => opts.retry_max_time = Some(n as u64),
                    Err(_) => {
                        eprintln!(
                            "curl: option --retry-max-time: expected a proper numerical parameter"
                        );
                        eprintln!(
                            "curl: try 'curl --help' or 'curl --manual' for more information"
                        );
                        process::exit(2);
                    }
                }
            }
            "--retry-delay" => {
                i += 1;
                let val = next_arg(&args, i, "--retry-delay");
                if val.parse::<u32>().is_err() {
                    eprintln!("curl: option --retry-delay: expected a proper numerical parameter");
                    eprintln!("curl: try 'curl --help' or 'curl --manual' for more information");
                    process::exit(2);
                }
            }
            "--retry-connrefused" | "--retry-all-errors" => {}
            "--expect100-timeout" => {
                i += 1;
                let _ = next_arg(&args, i, "--expect100-timeout");
            }
            "-r" | "--range" => {
                i += 1;
                opts.range = Some(next_arg(&args, i, "-r"));
            }
            "-T" | "--upload-file" => {
                i += 1;
                opts.upload_file = Some(PathBuf::from(next_arg(&args, i, "-T")));
            }
            "--http1.0" | "-0" => {
                opts.http_version = Some("1.0".into());
            }
            "--http1.1" => {
                opts.http_version = Some("1.1".into());
            }
            "--http0.9" => {
                opts.http09 = true;
            }
            "--remove-on-error" => {
                opts.remove_on_error = true;
            }
            "--raw" => {
                opts.raw = true;
            }
            "--etag-compare" => {
                i += 1;
                opts.etag_compare = Some(PathBuf::from(next_arg(&args, i, "--etag-compare")));
            }
            "--etag-save" => {
                i += 1;
                opts.etag_save = Some(PathBuf::from(next_arg(&args, i, "--etag-save")));
            }
            "--no-keepalive" => {
                opts.no_keepalive = true;
            }
            "--path-as-is" => {
                opts.path_as_is = true;
            }
            "--request-target" => {
                i += 1;
                opts.request_target = Some(next_arg(&args, i, "--request-target"));
            }
            "--cacert" => {
                i += 1;
                opts.cacert = Some(PathBuf::from(next_arg(&args, i, "--cacert")));
            }
            "--cert" | "-E" => {
                i += 1;
                opts.cert = Some(PathBuf::from(next_arg(&args, i, "--cert")));
            }
            "--key" => {
                i += 1;
                opts.cert_key = Some(PathBuf::from(next_arg(&args, i, "--key")));
            }
            "--url" => {
                i += 1;
                opts.urls.push(next_arg(&args, i, "--url"));
            }
            // Flags used by the curl test suite that we accept but ignore
            "-q" => {
                // Disable .curlrc — we don't read it anyway
            }
            "--trace-ascii" => {
                i += 1;
                let _path = next_arg(&args, i, "--trace-ascii");
                // TODO: implement trace output
            }
            "--trace-time" => {
                // Add timestamps to trace — ignored without trace support
            }
            "--trace" => {
                i += 1;
                let _path = next_arg(&args, i, "--trace");
                // TODO: implement trace output
            }
            "-n" | "--netrc" => {
                // Ignore netrc support
            }
            "--no-progress-meter" => {
                // We don't have a progress meter anyway
            }
            "--trace-config" => {
                i += 1;
                let _val = next_arg(&args, i, "--trace-config");
            }
            "-4" | "--ipv4" => {
                // Default behavior — we only support IPv4 anyway
            }
            "-6" | "--ipv6" => {
                // Ignored — we don't have IPv6-only mode
            }
            "--proto" => {
                i += 1;
                let _val = next_arg(&args, i, "--proto");
                // Ignored — we only support http/https
            }
            "--proto-redir" => {
                i += 1;
                let _val = next_arg(&args, i, "--proto-redir");
            }
            "-G" | "--get" => {
                opts.get = true;
            }
            "-g" | "--globoff" => {
                opts.globoff = true;
            }
            "-j" | "--junk-session-cookies" => {
                opts.junk_session_cookies = true;
            }
            "-C" | "--continue-at" => {
                i += 1;
                let val = next_arg(&args, i, "-C");
                opts.resume_from = Some(val);
            }
            "--form-string" => {
                i += 1;
                let val = next_arg(&args, i, "--form-string");
                // Like -F but don't interpret @ or < in value
                if let Some((name, rest)) = val.split_once('=') {
                    opts.form_fields.push(crate::options::FormField {
                        name: name.to_string(),
                        value: rest.to_string(),
                        is_file: false,
                        content_type: None,
                        filename: None,
                    });
                }
            }
            "--no-include" => {
                opts.include_headers = false;
            }
            "-N" | "--no-buffer" => {
                // Disable output buffering — ignored
            }
            "--resolve" => {
                i += 1;
                let val = next_arg(&args, i, "--resolve");
                opts.resolves.push(val);
            }
            "-K" | "--config" => {
                i += 1;
                let path = next_arg(&args, i, "-K");
                let file_args = read_config_file(&path);
                // Splice read args after the current position. They are picked
                // up by the next loop iterations and each is Unicode-checked.
                for a in &file_args {
                    check_unicode_warning(a);
                }
                for (j, a) in file_args.into_iter().enumerate() {
                    args.insert(i + 1 + j, a);
                }
            }
            "-x" | "--proxy" => {
                i += 1;
                let val = next_arg(&args, i, "-x");
                opts.proxy = Some(val);
            }
            "-U" | "--proxy-user" => {
                i += 1;
                let val = next_arg(&args, i, "-U");
                opts.proxy_user = Some(val);
            }
            "-p" | "--proxytunnel" => {
                opts.proxy_tunnel = true;
            }
            "--proxy1.0" => {
                i += 1;
                let val = next_arg(&args, i, "--proxy1.0");
                opts.proxy = Some(val);
                opts.proxy_1_0 = true;
            }
            "--anyauth" | "--digest" | "--ntlm" | "--negotiate" => {
                // We don't implement challenge/response auth. With these flags set,
                // curl waits for a 401 challenge before sending credentials. For
                // servers that don't challenge, the first request has no auth.
                opts.defer_auth = true;
            }
            "--basic" => {
                // Default — -u alone sends Basic auth on first request.
            }
            "-z" | "--time-cond" => {
                i += 1;
                let val = next_arg(&args, i, "-z");
                let (negate, date_str) = if let Some(rest) = val.strip_prefix('-') {
                    (true, rest.to_string())
                } else {
                    (false, val)
                };
                // Try as a file first (use its mtime)
                let timestamp = if let Ok(meta) = std::fs::metadata(&date_str) {
                    meta.modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0)
                } else {
                    // Parse date string - try common formats
                    parse_date_string(&date_str).unwrap_or(0)
                };
                if negate {
                    opts.time_cond = Some(crate::options::TimeCond::IfUnmodifiedSince(timestamp));
                } else {
                    opts.time_cond = Some(crate::options::TimeCond::IfModifiedSince(timestamp));
                }
            }
            "--stderr" => {
                i += 1;
                let val = next_arg(&args, i, "--stderr");
                opts.stderr_redirect = Some(std::path::PathBuf::from(val));
            }
            "--skip-existing" => {
                opts.skip_existing = true;
            }
            "--no-clobber" => {
                opts.no_clobber = true;
            }
            "--clobber" => {
                opts.no_clobber = false;
            }
            _ => {
                if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                    // Handle combined short flags like -sSL
                    let chars: Vec<char> = arg[1..].chars().collect();
                    let mut j = 0;
                    while j < chars.len() {
                        let flag = chars[j];
                        // Value-accepting short flags: consume rest of the cluster as
                        // the value if non-empty, else consume the next argv.
                        let takes_value = matches!(
                            flag,
                            'o' | 'X'
                                | 'H'
                                | 'd'
                                | 'u'
                                | 'A'
                                | 'e'
                                | 'b'
                                | 'c'
                                | 'F'
                                | 'D'
                                | 'w'
                                | 'r'
                                | 'T'
                                | 'm'
                                | 'E'
                                | 'z'
                                | 'x'
                                | 'U'
                        );
                        if takes_value {
                            let rest: String = chars[j + 1..].iter().collect();
                            let val = if rest.is_empty() {
                                i += 1;
                                next_arg(&args, i, &format!("-{flag}"))
                            } else {
                                rest
                            };
                            match flag {
                                'o' => opts.outputs.push(PathBuf::from(val)),
                                'X' => opts.method = Some(val),
                                'H' => parse_header_arg(&mut opts, &val),
                                'd' => append_data(&mut opts, &val, false),
                                'u' => opts.user = Some(val),
                                'A' => opts.user_agent = Some(val),
                                'e' => opts.referer = Some(val),
                                'b' => opts.cookies.push(val),
                                'c' => opts.cookie_jar = Some(PathBuf::from(val)),
                                'F' => parse_form_field(&mut opts, &val),
                                'D' => opts.dump_header = Some(PathBuf::from(val)),
                                'w' => opts.write_out = Some(val),
                                'r' => opts.range = Some(val),
                                'T' => opts.upload_file = Some(PathBuf::from(val)),
                                'm' => {
                                    let secs: f64 = val.parse().unwrap_or(0.0);
                                    opts.max_time = Some(Duration::from_secs_f64(secs));
                                }
                                'E' => opts.cert = Some(PathBuf::from(val)),
                                'z' => {
                                    let (negate, date_str) =
                                        if let Some(rest) = val.strip_prefix('-') {
                                            (true, rest.to_string())
                                        } else {
                                            (false, val)
                                        };
                                    let timestamp = if let Ok(meta) = std::fs::metadata(&date_str) {
                                        meta.modified()
                                            .ok()
                                            .and_then(|t| {
                                                t.duration_since(std::time::UNIX_EPOCH).ok()
                                            })
                                            .map(|d| d.as_secs() as i64)
                                            .unwrap_or(0)
                                    } else {
                                        parse_date_string(&date_str).unwrap_or(0)
                                    };
                                    if negate {
                                        opts.time_cond = Some(
                                            crate::options::TimeCond::IfUnmodifiedSince(timestamp),
                                        );
                                    } else {
                                        opts.time_cond = Some(
                                            crate::options::TimeCond::IfModifiedSince(timestamp),
                                        );
                                    }
                                }
                                'x' => opts.proxy = Some(val),
                                'U' => opts.proxy_user = Some(val),
                                _ => {}
                            }
                            j = chars.len();
                            continue;
                        }
                        match flag {
                            'v' => opts.verbose = true,
                            's' => opts.silent = true,
                            'S' => opts.show_error = true,
                            'f' => opts.fail = true,
                            'i' => opts.include_headers = true,
                            'I' => opts.head = true,
                            'L' => opts.location = true,
                            'k' => opts.insecure = true,
                            'O' => opts.remote_name = true,
                            'J' => opts.remote_header_name = true,
                            'n' => {} // --netrc, ignored
                            'q' => {} // disable .curlrc, ignored
                            'g' => opts.globoff = true,
                            'j' => opts.junk_session_cookies = true,
                            'G' => opts.get = true,
                            'N' => {} // --no-buffer, ignored
                            'p' => opts.proxy_tunnel = true,
                            '0' => opts.http_version = Some("1.0".into()),
                            c => {
                                eprintln!("curl: unknown option '-{c}'");
                                process::exit(2);
                            }
                        }
                        j += 1;
                    }
                } else if arg.starts_with("--") {
                    eprintln!("curl: unknown option '{arg}'");
                    process::exit(2);
                } else {
                    opts.urls.push(arg.clone());
                }
            }
        }
        i += 1;
    }

    if opts.urls.is_empty() {
        eprintln!("curl: no URL specified");
        process::exit(2);
    }

    // --continue-at is not compatible with request bodies (-d/--data* etc.).
    if opts.resume_from.is_some() && (opts.data.is_some() || !opts.form_fields.is_empty()) {
        eprintln!("curl: cannot mix --continue-at with --data");
        process::exit(2);
    }

    // --json: Content-Type / Accept defaults, applied once all -H args are
    // parsed so user-provided values are respected regardless of flag order.
    // curl emits these *after* any user-supplied headers.
    if opts.json {
        let has_ct = opts
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));
        let has_accept = opts
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("accept"));
        if !has_ct {
            opts.headers
                .push(("Content-Type".to_string(), "application/json".to_string()));
        }
        if !has_accept {
            opts.headers
                .push(("Accept".to_string(), "application/json".to_string()));
        }
    }

    // --etag-compare and --etag-save only make sense for a single URL; with
    // multiple URLs curl exits 2 with an explanatory 3-line warning block.
    if (opts.etag_compare.is_some() || opts.etag_save.is_some()) && opts.urls.len() > 1 {
        let opt = if opts.etag_save.is_some() {
            "--etag-save"
        } else {
            "--etag-compare"
        };
        eprintln!("curl: The etag options only work on a single URL");
        eprintln!("curl: option {opt}: is badly used here");
        eprintln!("curl: try 'curl --help' or 'curl --manual' for more information");
        process::exit(2);
    }

    // Mutually-exclusive HTTP request methods. curl accepts at most one of
    // -I (HEAD), -T (PUT), and -d/-F (POST). With -G the -d data is promoted
    // to a query string rather than a body, so it doesn't count as POST.
    let method_labels: &[(bool, &str)] = &[
        (opts.head, "HEAD (-I, --head)"),
        (opts.upload_file.is_some(), "PUT (-T, --upload-file)"),
        (opts.data.is_some() && !opts.get, "POST (-d, --data)"),
        (!opts.form_fields.is_empty(), "POST (-F, --form)"),
    ];
    let chosen: Vec<&str> = method_labels
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, l)| *l)
        .collect();
    if chosen.len() > 1 {
        // Emit the first pair of conflicts; curl's phrasing is "both A and B".
        let a = chosen[0];
        let b = chosen[1];
        warn_wrapped(&format!(
            "You can only select one HTTP request method! You asked for both {a} and {b}."
        ));
        process::exit(2);
    }

    // Mutual exclusion: --continue-at vs --no-clobber
    if opts.no_clobber && opts.resume_from.is_some() {
        eprintln!("curl: --continue-at is mutually exclusive with --no-clobber");
        eprintln!("curl: option -C: is badly used here");
        eprintln!("curl: try 'curl --help' or 'curl --manual' for more information");
        process::exit(2);
    }
    // Mutual exclusion: --continue-at vs --remove-on-error
    if opts.remove_on_error && opts.resume_from.is_some() {
        eprintln!("curl: --continue-at is mutually exclusive with --remove-on-error");
        eprintln!("curl: option -C: is badly used here");
        eprintln!("curl: try 'curl --help' or 'curl --manual' for more information");
        process::exit(2);
    }
    opts
}

/// Handle a -H argument, which may be a header string or "@file" to read
/// one header per line from a file.
fn parse_header_arg(opts: &mut Options, val: &str) {
    if let Some(path) = val.strip_prefix('@') {
        if let Ok(contents) = fs::read_to_string(path) {
            for line in contents.split('\n') {
                let line = line.trim_end_matches('\r');
                if line.is_empty() {
                    continue;
                }
                // Preserve the line verbatim — curl emits file-loaded header
                // lines unchanged (including leading/surrounding whitespace).
                // Only handle the "Name;" no-value marker and colon-based split,
                // but keep original spacing on either side.
                if let Some(k) = line.strip_suffix(';') {
                    opts.headers.push((k.to_string(), "\x00".to_string()));
                } else if let Some((k, v)) = line.split_once(':') {
                    // Store key with its original whitespace; value without
                    // a leading space (curl emits exactly one space after the
                    // colon when re-serializing).
                    let v = v.strip_prefix(' ').unwrap_or(v);
                    opts.headers.push((k.to_string(), v.to_string()));
                }
            }
        }
    } else {
        parse_custom_header(opts, val);
    }
}

/// Parse a custom -H header string.
/// - "Name: value" → adds header
/// - "Name:" → empty value (suppresses/removes default header)
/// - "Name;" → sends header with no colon-value (like "Name:\r\n")
fn parse_custom_header(opts: &mut Options, h: &str) {
    if let Some((k, v)) = h.split_once(':') {
        // "Name: value" or "Name:" (empty value)
        opts.headers
            .push((k.trim().to_string(), v.trim_start().to_string()));
    } else if let Some(k) = h.strip_suffix(';') {
        // "Name;" — send header with empty value (no-value header)
        // Store with a special marker so request.rs sends "Name:\r\n"
        opts.headers
            .push((k.trim().to_string(), "\x00".to_string()));
    }
}

fn next_arg(args: &[String], i: usize, flag: &str) -> String {
    if i >= args.len() {
        eprintln!("curl: option {flag} requires an argument");
        process::exit(2);
    }
    args[i].clone()
}

fn append_data(opts: &mut Options, val: &str, raw: bool) {
    let data = if let Some(path) = (!raw).then_some(()).and_then(|()| val.strip_prefix('@')) {
        if path == "-" {
            let mut buf = String::new();
            let _ = io::stdin().read_to_string(&mut buf);
            buf.into_bytes()
                .into_iter()
                .filter(|b| *b != b'\r' && *b != b'\n' && *b != b'\0')
                .collect()
        } else {
            match fs::read(path) {
                Ok(d) => d
                    .into_iter()
                    .filter(|b| *b != b'\r' && *b != b'\n' && *b != b'\0')
                    .collect(),
                Err(e) => {
                    eprintln!("curl: failed to read {path}: {e}");
                    process::exit(2);
                }
            }
        }
    } else {
        val.as_bytes().to_vec()
    };

    match opts.data {
        Some(ref mut existing) => {
            existing.push(b'&');
            existing.extend_from_slice(&data);
        }
        None => {
            opts.data = Some(data);
        }
    }
}

fn append_data_binary(opts: &mut Options, val: &str) {
    let data = if let Some(path) = val.strip_prefix('@') {
        if path == "-" {
            let mut buf = Vec::new();
            let _ = io::stdin().read_to_end(&mut buf);
            buf
        } else {
            match fs::read(path) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("curl: failed to read {path}: {e}");
                    process::exit(2);
                }
            }
        }
    } else {
        val.as_bytes().to_vec()
    };

    match opts.data {
        Some(ref mut existing) => {
            existing.extend_from_slice(&data);
        }
        None => {
            opts.data = Some(data);
        }
    }
}

fn parse_form_field(opts: &mut Options, val: &str) {
    if let Some((name, rest)) = val.split_once('=') {
        if let Some(file_part) = rest.strip_prefix('@') {
            // File upload: @path or @"path" with optional ;type= and ;filename=
            // modifiers.  Modifier values extend until the next recognized
            // modifier marker, so they can contain ';'.
            let (path, content_type, filename) = split_file_form_modifiers(file_part);

            opts.form_fields.push(FormField {
                name: name.to_string(),
                value: path,
                is_file: true,
                content_type,
                filename,
            });
        } else if let Some(file_part) = rest.strip_prefix('<') {
            // Read file contents as the field value (NOT a file upload).
            // Supports the same ;type= modifier syntax.
            let (path, content_type, _filename) = split_file_form_modifiers(file_part);
            let contents = if path == "-" {
                let mut buf = String::new();
                let _ = io::stdin().read_to_string(&mut buf);
                buf
            } else {
                match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("curl: failed to read {path}: {e}");
                        process::exit(2);
                    }
                }
            };
            opts.form_fields.push(FormField {
                name: name.to_string(),
                value: contents,
                is_file: false,
                content_type,
                filename: None,
            });
        } else {
            // Text field may have ;type=... modifier that applies a Content-Type.
            // Do NOT split on every ';' — the type value itself can contain ';'
            // (e.g. "text/html;charset=verymoo"). Only split once on ";type=".
            // curl also trims a single leading space after '=' in the value.
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            let mut value = rest.to_string();
            let mut content_type = None;
            if let Some(idx) = find_type_modifier(rest) {
                value = rest[..idx].to_string();
                content_type = Some(rest[idx + ";type=".len()..].to_string());
            }
            opts.form_fields.push(FormField {
                name: name.to_string(),
                value,
                is_file: false,
                content_type,
                filename: None,
            });
        }
    }
}

fn find_type_modifier(s: &str) -> Option<usize> {
    s.find(";type=")
}

/// Split "path;type=X;filename=Y" into (path, type, filename).
///
/// Supports curl's quoting:
///   - Quoted paths:    `"path/to file";type=X`
///   - Quoted values:   `path;filename="my file.txt"`
///   - Space after `;`: `path; type=X; filename=Y`
///   - Escape in quotes: `\X` → `X` for any character X
///
/// For unquoted modifier values, the value extends until the next recognized
/// modifier marker (`;type=` / `;filename=`), so values can contain bare `;`
/// (e.g. `type=text/html;charset=utf-8`).
fn split_file_form_modifiers(s: &str) -> (String, Option<String>, Option<String>) {
    let bytes = s.as_bytes();

    // --- Parse the path (may be quoted) ---
    let (path, mut pos) = if bytes.first() == Some(&b'"') {
        let (val, end) = form_read_quoted(s, 1); // skip opening quote
        (val, end)
    } else {
        // Unquoted path — ends at first modifier marker (;type= / ;filename=)
        // so that paths without modifiers are taken in full.
        let end = form_find_modifier_pos(s, 0).unwrap_or(s.len());
        (s[..end].to_string(), end)
    };

    // --- Parse modifiers ---
    let mut content_type = None;
    let mut filename = None;
    while pos < s.len() {
        if bytes[pos] == b';' {
            pos += 1;
            // Skip optional whitespace after ';'
            while pos < s.len() && bytes[pos] == b' ' {
                pos += 1;
            }
            let rest = &s[pos..];
            if rest.len() >= 5 && rest[..5].eq_ignore_ascii_case("type=") {
                pos += 5;
                let (val, end) = form_read_modifier_value(s, pos);
                pos = end;
                content_type = Some(val);
            } else if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("filename=") {
                pos += 9;
                let (val, end) = form_read_modifier_value(s, pos);
                pos = end;
                filename = Some(val);
            } else {
                // Unknown modifier — skip to next ';' or end
                let next = s[pos..].find(';').map(|i| pos + i).unwrap_or(s.len());
                pos = next;
            }
        } else {
            pos += 1;
        }
    }

    (path, content_type, filename)
}

/// Read a double-quoted value starting just after the opening `"`.
/// Only `\\` → `\` and `\"` → `"` are recognized escape sequences
/// (matching curl's `get_param_word`).  All other `\X` are kept verbatim.
/// Returns (unescaped value, position after closing `"`).
fn form_read_quoted(s: &str, start: usize) -> (String, usize) {
    let bytes = s.as_bytes();
    let mut result = String::new();
    let mut pos = start;
    while pos < bytes.len() {
        if bytes[pos] == b'\\' && pos + 1 < bytes.len() {
            let next = bytes[pos + 1];
            if next == b'\\' || next == b'"' {
                // Recognized escape: consume backslash, keep the next char
                pos += 1;
                result.push(bytes[pos] as char);
                pos += 1;
            } else {
                // Not a recognized escape — keep the backslash literally
                result.push(bytes[pos] as char);
                pos += 1;
            }
        } else if bytes[pos] == b'"' {
            pos += 1; // skip closing quote
            return (result, pos);
        } else {
            result.push(bytes[pos] as char);
            pos += 1;
        }
    }
    // No closing quote — return what we have
    (result, pos)
}

/// Read a modifier value that may be quoted (`"..."`) or unquoted.
/// Unquoted values extend to the next recognized modifier marker or end.
fn form_read_modifier_value(s: &str, start: usize) -> (String, usize) {
    let bytes = s.as_bytes();
    if start < bytes.len() && bytes[start] == b'"' {
        form_read_quoted(s, start + 1)
    } else {
        // Unquoted — extends to next modifier start (`;type=` / `;filename=`)
        // so that values can contain bare ';' (e.g. "text/html;charset=utf-8").
        let end = form_find_modifier_pos(s, start).unwrap_or(s.len());
        (s[start..end].to_string(), end)
    }
}

/// Find the byte offset of the next `;` that introduces a recognized modifier
/// (`;type=` or `;filename=`, with optional spaces after the `;`).
fn form_find_modifier_pos(s: &str, from: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut pos = from;
    while pos < bytes.len() {
        if bytes[pos] == b';' {
            let mut check = pos + 1;
            while check < bytes.len() && bytes[check] == b' ' {
                check += 1;
            }
            let rest = &s[check..];
            if (rest.len() >= 5 && rest[..5].eq_ignore_ascii_case("type="))
                || (rest.len() >= 9 && rest[..9].eq_ignore_ascii_case("filename="))
            {
                return Some(pos);
            }
        }
        pos += 1;
    }
    None
}

/// Parse a date string into a Unix timestamp. Supports:
/// - Epoch seconds (plain integer)
/// - RFC 2616 / HTTP date: "Sun, 06 Nov 1994 08:49:37 GMT"
/// - ISO 8601: "1994-11-06T08:49:37Z" or "1994-11-06 08:49:37"
/// - Common formats: "Nov 6, 1994", "6 Nov 1994", "December 12, 2003"
pub(crate) fn parse_date_string(s: &str) -> Option<i64> {
    let s = s.trim();
    // Try epoch seconds
    if let Ok(n) = s.parse::<i64>() {
        return Some(n);
    }

    let months: [(&str, u32); 12] = [
        ("jan", 1),
        ("feb", 2),
        ("mar", 3),
        ("apr", 4),
        ("may", 5),
        ("jun", 6),
        ("jul", 7),
        ("aug", 8),
        ("sep", 9),
        ("oct", 10),
        ("nov", 11),
        ("dec", 12),
    ];

    fn month_num(s: &str, months: &[(&str, u32)]) -> Option<u32> {
        let lower = s.to_lowercase();
        months
            .iter()
            .find(|(name, _)| lower.starts_with(name))
            .map(|(_, n)| *n)
    }

    fn to_epoch(year: i64, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> i64 {
        // Simple epoch calculation
        let mut y = year;
        let mut m = month as i64;
        if m <= 2 {
            y -= 1;
            m += 12;
        }
        let days =
            365 * y + y / 4 - y / 100 + y / 400 + (153 * (m - 3) + 2) / 5 + day as i64 - 719469;
        days * 86400 + hour as i64 * 3600 + min as i64 * 60 + sec as i64
    }

    // Try ISO 8601: "1994-11-06T08:49:37" or "1994-11-06 08:49:37"
    if s.len() >= 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
        let year: i64 = s[0..4].parse().ok()?;
        let month: u32 = s[5..7].parse().ok()?;
        let day: u32 = s[8..10].parse().ok()?;
        let (hour, min, sec) = if s.len() >= 19 {
            let t = &s[11..19];
            let h: u32 = t[0..2].parse().ok()?;
            let m: u32 = t[3..5].parse().ok()?;
            let s: u32 = t[6..8].parse().ok()?;
            (h, m, s)
        } else {
            (0, 0, 0)
        };
        return Some(to_epoch(year, month, day, hour, min, sec));
    }

    // Try RFC 2616: "Sun, 06 Nov 1994 08:49:37 GMT"
    // Also: "Sunday, 06-Nov-94 08:49:37 GMT" (RFC 850)
    let parts: Vec<&str> = s.split([' ', ',', '-']).filter(|p| !p.is_empty()).collect();
    if parts.len() >= 4 {
        // Skip day-of-week if present
        let start = if parts[0].len() >= 3
            && months
                .iter()
                .all(|(m, _)| !parts[0].to_lowercase().starts_with(m))
            && parts[0].chars().next().is_some_and(|c| c.is_alphabetic())
        {
            1
        } else {
            0
        };

        // Try "DD Mon YYYY HH:MM:SS"
        if let Some(day) = parts.get(start).and_then(|s| s.parse::<u32>().ok())
            && let Some(mon) = parts.get(start + 1).and_then(|s| month_num(s, &months))
            && let Some(year_str) = parts.get(start + 2)
        {
            let mut year: i64 = year_str.parse().ok()?;
            if year < 100 {
                year += if year < 70 { 2000 } else { 1900 };
            }
            let (hour, min, sec) = if let Some(time_str) = parts.get(start + 3) {
                if time_str.contains(':') {
                    let tp: Vec<&str> = time_str.split(':').collect();
                    let h: u32 = tp.first()?.parse().ok()?;
                    let m: u32 = tp.get(1).unwrap_or(&"0").parse().ok()?;
                    let s: u32 = tp.get(2).unwrap_or(&"0").parse().ok()?;
                    (h, m, s)
                } else {
                    (0, 0, 0)
                }
            } else {
                (0, 0, 0)
            };
            return Some(to_epoch(year, mon, day, hour, min, sec));
        }

        // Try "Mon DD, YYYY" or "Month DD, YYYY" or "Mon DD HH:MM:SS YYYY"
        // (e.g. "Dec 12 12:00:00 1999 GMT"; trailing "GMT" ignored).
        if let Some(mon) = month_num(parts[start], &months)
            && let Some(day) = parts.get(start + 1).and_then(|s| s.parse::<u32>().ok())
        {
            // Look at part[start+2]: time first, or year first?
            let next = parts.get(start + 2)?;
            if next.contains(':') {
                let tp: Vec<&str> = next.split(':').collect();
                let h: u32 = tp.first()?.parse().ok()?;
                let m: u32 = tp.get(1).unwrap_or(&"0").parse().ok()?;
                let s: u32 = tp.get(2).unwrap_or(&"0").parse().ok()?;
                let year: i64 = parts.get(start + 3)?.parse().ok()?;
                return Some(to_epoch(year, mon, day, h, m, s));
            }
            let year: i64 = next.parse().ok()?;
            let (hour, min, sec) = if let Some(time_str) = parts.get(start + 3) {
                if time_str.contains(':') {
                    let tp: Vec<&str> = time_str.split(':').collect();
                    let h: u32 = tp.first()?.parse().ok()?;
                    let m: u32 = tp.get(1).unwrap_or(&"0").parse().ok()?;
                    let s: u32 = tp.get(2).unwrap_or(&"0").parse().ok()?;
                    (h, m, s)
                } else {
                    (0, 0, 0)
                }
            } else {
                (0, 0, 0)
            };
            return Some(to_epoch(year, mon, day, hour, min, sec));
        }
    }

    None
}

pub(crate) fn format_http_date(timestamp: i64) -> String {
    let days = timestamp.div_euclid(86400);
    let secs_in_day = timestamp.rem_euclid(86400);
    let hour = secs_in_day / 3600;
    let min = (secs_in_day % 3600) / 60;
    let sec = secs_in_day % 60;

    // Day of week (0=Thursday for epoch)
    let dow = ((days + 4) % 7 + 7) % 7; // 0=Sun, 1=Mon, ...
    let dow_names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let dow_name = dow_names[dow as usize % 7];

    // Civil date from days since epoch
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    let month_names = [
        "", "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        dow_name, d, month_names[m as usize], y, hour, min, sec
    )
}

fn print_usage() {
    println!(
        "\
Usage: curl [options...] <url>

Options:
  -X, --request <method>    HTTP method (GET, POST, PUT, DELETE, etc.)
  -H, --header <header>     Custom header (e.g. \"Content-Type: application/json\")
  -d, --data <data>         POST data (use @file to read from file)
  --data-raw <data>         POST data without @ file interpretation
  --data-binary <data>      POST binary data
  --data-urlencode <data>   URL-encode POST data
  -F, --form <name=value>   Multipart form data (use @file for file upload)
  -o, --output <file>       Write output to file
  -O, --remote-name         Write output to file named from URL
  -L, --location            Follow redirects
  --max-redirs <num>        Maximum number of redirects (default: 50)
  -v, --verbose             Verbose output
  -s, --silent              Silent mode
  -S, --show-error          Show errors in silent mode
  -f, --fail                Fail silently on HTTP errors
  -i, --include             Include response headers in output
  -I, --head                HEAD request (headers only)
  -A, --user-agent <agent>  User-Agent header
  -e, --referer <url>       Referer header
  -b, --cookie <data|file>  Send cookies
  -c, --cookie-jar <file>   Save cookies to file
  -u, --user <user:pass>    Basic authentication
  --connect-timeout <secs>  Connection timeout
  -m, --max-time <secs>     Maximum transfer time
  -k, --insecure            Skip TLS verification
  --compressed              Request compressed response
  -D, --dump-header <file>  Dump headers to file
  -w, --write-out <format>  Output format after transfer
  --retry <num>             Retry count on failure
  -r, --range <range>       Byte range (e.g. 0-499)
  -T, --upload-file <file>  Upload file (PUT)
  -0, --http1.0             Use HTTP/1.0
  --http1.1                 Use HTTP/1.1
  --no-keepalive            Disable keepalive
  --cacert <file>           CA certificate bundle
  -E, --cert <file>         Client certificate
  --key <file>              Client private key
  --url <url>               Explicit URL
  -h, --help                Show this help
  -z, --time-cond <time>    Transfer based on time condition
  -V, --version             Show version"
    );
}

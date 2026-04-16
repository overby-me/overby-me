use std::io::{self, Read};
use std::path::PathBuf;
use std::time::Duration;
use std::{env, fs, process};

use crate::format::urlencode_field;
use crate::options::{FormField, Options};

pub(crate) fn parse_args() -> Options {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut opts = Options::default();

    if args.is_empty() {
        print_usage();
        process::exit(0);
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
                println!("Features: HTTPS SSL");
                process::exit(0);
            }
            "-X" | "--request" => {
                i += 1;
                opts.method = Some(next_arg(&args, i, "-X"));
            }
            "-H" | "--header" => {
                i += 1;
                let h = next_arg(&args, i, "-H");
                if let Some((k, v)) = h.split_once(':') {
                    opts.headers
                        .push((k.trim().to_string(), v.trim_start().to_string()));
                }
            }
            "-d" | "--data" | "--data-ascii" => {
                i += 1;
                let val = next_arg(&args, i, "-d");
                append_data(&mut opts, &val, false);
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
                opts.output = Some(PathBuf::from(next_arg(&args, i, "-o")));
            }
            "-O" | "--remote-name" => {
                opts.remote_name = true;
            }
            "-L" | "--location" => {
                opts.location = true;
            }
            "--max-redirs" => {
                i += 1;
                opts.max_redirs = next_arg(&args, i, "--max-redirs").parse().unwrap_or(50);
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
                opts.fail = true;
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
                opts.cookie = Some(next_arg(&args, i, "-b"));
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
            "--no-keepalive" => {
                opts.no_keepalive = true;
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
            _ => {
                if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                    // Handle combined short flags like -sSL
                    let chars: Vec<char> = arg[1..].chars().collect();
                    let mut j = 0;
                    while j < chars.len() {
                        match chars[j] {
                            'v' => opts.verbose = true,
                            's' => opts.silent = true,
                            'S' => opts.show_error = true,
                            'f' => opts.fail = true,
                            'i' => opts.include_headers = true,
                            'I' => opts.head = true,
                            'L' => opts.location = true,
                            'k' => opts.insecure = true,
                            'O' => opts.remote_name = true,
                            'n' => {} // --netrc, ignored
                            'q' => {} // disable .curlrc, ignored
                            '0' => opts.http_version = Some("1.0".into()),
                            // Flags that consume the rest or next arg.
                            'o' => {
                                let rest: String = chars[j + 1..].iter().collect();
                                if rest.is_empty() {
                                    i += 1;
                                    opts.output = Some(PathBuf::from(next_arg(&args, i, "-o")));
                                } else {
                                    opts.output = Some(PathBuf::from(rest));
                                }
                                j = chars.len(); // consumed rest
                                continue;
                            }
                            'X' => {
                                let rest: String = chars[j + 1..].iter().collect();
                                if rest.is_empty() {
                                    i += 1;
                                    opts.method = Some(next_arg(&args, i, "-X"));
                                } else {
                                    opts.method = Some(rest);
                                }
                                j = chars.len();
                                continue;
                            }
                            'H' => {
                                i += 1;
                                let h = next_arg(&args, i, "-H");
                                if let Some((k, v)) = h.split_once(':') {
                                    opts.headers
                                        .push((k.trim().to_string(), v.trim().to_string()));
                                }
                                j = chars.len();
                                continue;
                            }
                            'd' => {
                                i += 1;
                                let val = next_arg(&args, i, "-d");
                                append_data(&mut opts, &val, false);
                                j = chars.len();
                                continue;
                            }
                            'u' => {
                                i += 1;
                                opts.user = Some(next_arg(&args, i, "-u"));
                                j = chars.len();
                                continue;
                            }
                            'A' => {
                                i += 1;
                                opts.user_agent = Some(next_arg(&args, i, "-A"));
                                j = chars.len();
                                continue;
                            }
                            'e' => {
                                i += 1;
                                opts.referer = Some(next_arg(&args, i, "-e"));
                                j = chars.len();
                                continue;
                            }
                            'b' => {
                                i += 1;
                                opts.cookie = Some(next_arg(&args, i, "-b"));
                                j = chars.len();
                                continue;
                            }
                            'c' => {
                                i += 1;
                                opts.cookie_jar = Some(PathBuf::from(next_arg(&args, i, "-c")));
                                j = chars.len();
                                continue;
                            }
                            'F' => {
                                i += 1;
                                let val = next_arg(&args, i, "-F");
                                parse_form_field(&mut opts, &val);
                                j = chars.len();
                                continue;
                            }
                            'D' => {
                                i += 1;
                                opts.dump_header = Some(PathBuf::from(next_arg(&args, i, "-D")));
                                j = chars.len();
                                continue;
                            }
                            'w' => {
                                i += 1;
                                opts.write_out = Some(next_arg(&args, i, "-w"));
                                j = chars.len();
                                continue;
                            }
                            'r' => {
                                i += 1;
                                opts.range = Some(next_arg(&args, i, "-r"));
                                j = chars.len();
                                continue;
                            }
                            'T' => {
                                i += 1;
                                opts.upload_file = Some(PathBuf::from(next_arg(&args, i, "-T")));
                                j = chars.len();
                                continue;
                            }
                            'm' => {
                                i += 1;
                                let secs: f64 = next_arg(&args, i, "-m").parse().unwrap_or(0.0);
                                opts.max_time = Some(Duration::from_secs_f64(secs));
                                j = chars.len();
                                continue;
                            }
                            'E' => {
                                i += 1;
                                opts.cert = Some(PathBuf::from(next_arg(&args, i, "-E")));
                                j = chars.len();
                                continue;
                            }
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

    opts
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
            let mut path = file_part.to_string();
            let mut content_type = None;
            let mut filename = None;

            // Parse ;type= and ;filename= modifiers.
            if let Some(semicolon) = file_part.find(';') {
                path = file_part[..semicolon].to_string();
                for modifier in file_part[semicolon + 1..].split(';') {
                    let modifier = modifier.trim();
                    if let Some(ct) = modifier.strip_prefix("type=") {
                        content_type = Some(ct.to_string());
                    } else if let Some(fn_) = modifier.strip_prefix("filename=") {
                        filename = Some(fn_.to_string());
                    }
                }
            }

            opts.form_fields.push(FormField {
                name: name.to_string(),
                value: path,
                is_file: true,
                content_type,
                filename,
            });
        } else {
            opts.form_fields.push(FormField {
                name: name.to_string(),
                value: rest.to_string(),
                is_file: false,
                content_type: None,
                filename: None,
            });
        }
    }
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
  -V, --version             Show version"
    );
}

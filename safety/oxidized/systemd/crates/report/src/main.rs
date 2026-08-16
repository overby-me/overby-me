//! `systemd-report` - acquire metrics from local sources.
//!
//! A faithful port of upstream `src/report/report.c`. It discovers metrics
//! sources (varlink sockets) under `/run/systemd/report/` (or, in `--user`
//! mode, `$XDG_RUNTIME_DIR/systemd/report/`), queries each via the
//! `io.systemd.Metrics.List` / `io.systemd.Metrics.Describe` varlink methods,
//! and prints the aggregated metrics as a table or JSON. `list-sources` shows
//! the discovered sources.

use std::io::{Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

// ── CLI options ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    System,
    User,
}

struct Options {
    scope: Scope,
    legend: bool,
    json: bool,
    matches: Vec<String>,
}

enum Verb {
    Help,
    Metrics,
    DescribeMetrics,
    ListSources,
}

// ── Name validators (mirror sd-varlink-idl.c + report.c) ────────────────────

/// Port of `varlink_idl_interface_name_is_valid`: dot-separated labels of
/// `[A-Za-z0-9]` (single `-` allowed inside), first char a letter, no empty
/// labels, no trailing `.`/`-`.
fn interface_name_is_valid(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes = name.as_bytes();
    if !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    let mut prev = b'a'; // first char already checked as a letter
    for (i, &c) in bytes.iter().enumerate() {
        if i == 0 {
            prev = c;
            continue;
        }
        if c == b'.' || c == b'-' {
            if prev == b'.' || prev == b'-' {
                return false;
            }
        } else if !c.is_ascii_alphanumeric() {
            return false;
        }
        prev = c;
    }
    let last = bytes[bytes.len() - 1];
    last != b'.' && last != b'-'
}

/// Port of `varlink_idl_field_name_is_valid`: first char a letter, rest
/// `[A-Za-z0-9]` or single `_`, no double/trailing underscore.
fn field_name_is_valid(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let bytes = name.as_bytes();
    if !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    let mut prev = bytes[0];
    for &c in &bytes[1..] {
        if c == b'_' {
            if prev == b'_' {
                return false;
            }
        } else if !c.is_ascii_alphanumeric() {
            return false;
        }
        prev = c;
    }
    bytes[bytes.len() - 1] != b'_'
}

/// Port of `metrics_name_valid`: `<interface>.<field>`. No dot -> not valid
/// (but not an error).
fn metrics_name_valid(name: &str) -> bool {
    match name.rfind('.') {
        None => false,
        Some(pos) => interface_name_is_valid(&name[..pos]) && field_name_is_valid(&name[pos + 1..]),
    }
}

/// Port of `metric_startswith_prefix`: `prefix` is a dotted prefix of `name`.
fn metric_startswith_prefix(name: &str, prefix: &str) -> bool {
    match name.strip_prefix(prefix) {
        Some(rest) => rest.starts_with('.'),
        None => false,
    }
}

// ── Source discovery ────────────────────────────────────────────────────────

fn sources_dir(scope: Scope) -> Option<PathBuf> {
    match scope {
        Scope::System => Some(PathBuf::from("/run/systemd/report")),
        Scope::User => {
            let rt = std::env::var_os("XDG_RUNTIME_DIR")?;
            Some(Path::new(&rt).join("systemd/report"))
        }
    }
}

/// A discovered source: its interface name (== filename) and socket path.
struct Source {
    name: String,
    path: PathBuf,
}

/// Enumerate metrics sources, applying the interface-name + service-match
/// filters. Returns an empty vec when the directory is absent (no error).
fn readdir_sources(opts: &Options) -> Vec<Source> {
    let mut out = Vec::new();
    let dir = match sources_dir(opts.scope) {
        Some(d) => d,
        None => return out,
    };
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return out, // ENOENT and friends -> no sources
    };
    for ent in rd.flatten() {
        let name = match ent.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        // Only sockets and symlinks (to sockets).
        let ft = match ent.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !(ft.is_socket() || ft.is_symlink()) {
            continue;
        }
        if !interface_name_is_valid(&name) {
            continue;
        }
        if !test_service_matches(&opts.matches, &name) {
            continue;
        }
        out.push(Source {
            name,
            path: ent.path(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Service-level prefilter: keep a source whose interface name is comparable
/// (a dotted prefix either way, or equal) to at least one requested match.
fn test_service_matches(matches: &[String], service: &str) -> bool {
    if matches.is_empty() {
        return true;
    }
    matches.iter().any(|m| {
        service == m || metric_startswith_prefix(m, service) || metric_startswith_prefix(service, m)
    })
}

// ── Varlink client (NUL-framed JSON over a unix socket) ─────────────────────

/// Issue a streaming `io.systemd.Metrics.List`/`Describe` call and collect the
/// per-reply metric objects. Best-effort: a source that fails to answer simply
/// contributes nothing.
fn metrics_call(source: &Source, method: &str) -> Vec<serde_json::Value> {
    let mut metrics = Vec::new();
    let stream = match UnixStream::connect(&source.path) {
        Ok(s) => s,
        Err(_) => return metrics,
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));

    let request = serde_json::json!({
        "method": method,
        "parameters": {},
        "more": true,
    });
    let mut msg = serde_json::to_vec(&request).unwrap_or_default();
    msg.push(0);
    {
        let mut w = &stream;
        if w.write_all(&msg).is_err() {
            return metrics;
        }
    }

    // Read NUL-framed replies until one lacks "continues": true.
    let mut reader = std::io::BufReader::new(&stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let mut byte = [0u8; 1];
        let mut got = false;
        loop {
            match reader.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    got = true;
                    if byte[0] == 0 {
                        break;
                    }
                    buf.push(byte[0]);
                }
                Err(_) => break,
            }
        }
        if !got || buf.is_empty() {
            break;
        }
        let reply: serde_json::Value = match serde_json::from_slice(&buf) {
            Ok(v) => v,
            Err(_) => break,
        };
        if reply.get("error").is_some() {
            break; // varlink error -> ignore this source's remaining data
        }
        if let Some(params) = reply.get("parameters") {
            // A metric's name must have the source's interface name as a dotted
            // prefix; the user-level `matches` filter is applied at output time.
            if let Some(name) = params.get("name").and_then(|v| v.as_str())
                && metric_startswith_prefix(name, &source.name)
            {
                metrics.push(params.clone());
            }
        }
        let continues = reply
            .get("continues")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !continues {
            break;
        }
    }
    metrics
}

/// Whether a metric `name` passes the user's `matches` filter.
fn metric_matches(matches: &[String], name: &str) -> bool {
    if matches.is_empty() {
        return true;
    }
    matches
        .iter()
        .any(|m| name == m || metric_startswith_prefix(name, m))
}

// ── Verbs ───────────────────────────────────────────────────────────────────

fn verb_metrics(opts: &Options, describe: bool) -> ExitCode {
    let method = if describe {
        "io.systemd.Metrics.Describe"
    } else {
        "io.systemd.Metrics.List"
    };
    let sources = readdir_sources(opts);
    if sources.is_empty() {
        if opts.legend {
            eprintln!("No metrics sources found.");
        }
        return ExitCode::SUCCESS;
    }

    let mut collected: Vec<serde_json::Value> = Vec::new();
    for src in &sources {
        for m in metrics_call(src, method) {
            if let Some(name) = m.get("name").and_then(|v| v.as_str())
                && metric_matches(&opts.matches, name)
            {
                collected.push(m);
            }
        }
    }

    collected.sort_by(|a, b| {
        let an = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let bn = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
        an.cmp(bn)
    });

    if opts.json {
        // application/json-seq: each object preceded by RS (0x1E), one per line.
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        for m in &collected {
            let _ = lock.write_all(&[0x1e]);
            let _ = writeln!(lock, "{}", serde_json::to_string(m).unwrap_or_default());
        }
        return ExitCode::SUCCESS;
    }

    if collected.is_empty() {
        if opts.legend {
            println!("No metrics available.");
        }
        return ExitCode::SUCCESS;
    }

    print_metrics_table(&collected, describe, opts.legend);
    ExitCode::SUCCESS
}

fn print_metrics_table(metrics: &[serde_json::Value], describe: bool, legend: bool) {
    let field = |m: &serde_json::Value, k: &str| -> String {
        match m.get(k) {
            None | Some(serde_json::Value::Null) => "-".to_string(),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
        }
    };
    if describe {
        if legend {
            println!("{:<40} {:<10} DESCRIPTION", "FAMILY", "TYPE");
        }
        for m in metrics {
            println!(
                "{:<40} {:<10} {}",
                field(m, "name"),
                field(m, "type"),
                field(m, "description")
            );
        }
    } else {
        if legend {
            println!("{:<40} {:<20} {:<20} VALUE", "FAMILY", "OBJECT", "FIELDS");
        }
        for m in metrics {
            println!(
                "{:<40} {:<20} {:<20} {}",
                field(m, "name"),
                field(m, "object"),
                field(m, "fields"),
                field(m, "value")
            );
        }
    }
    if legend {
        println!("\n{} metrics listed.", metrics.len());
    }
}

fn verb_list_sources(opts: &Options) -> ExitCode {
    let sources = readdir_sources(opts);

    if opts.json {
        let arr: Vec<serde_json::Value> = sources
            .iter()
            .map(|s| {
                let resolved = std::fs::canonicalize(&s.path).unwrap_or_else(|_| s.path.clone());
                serde_json::json!({
                    "source": s.name,
                    "address": format!("unix:{}", resolved.display()),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
        );
        return ExitCode::SUCCESS;
    }

    if sources.is_empty() {
        if opts.legend {
            println!("No sources available.");
        }
        return ExitCode::SUCCESS;
    }

    if opts.legend {
        println!("{:<30} ADDRESS", "SOURCE");
    }
    for s in &sources {
        let resolved = std::fs::canonicalize(&s.path).unwrap_or_else(|_| s.path.clone());
        println!("{:<30} unix:{}", s.name, resolved.display());
    }
    if opts.legend {
        println!("\n{} sources listed.", sources.len());
    }
    ExitCode::SUCCESS
}

fn help() {
    println!(
        "systemd-report [OPTIONS...] COMMAND ...\n\n\
         Acquire metrics from local sources.\n\n\
         Commands:\n  \
         metrics [MATCH...]    Acquire list of metrics and their values\n  \
         describe-metrics [MATCH...]\n                        Describe available metrics\n  \
         list-sources          Show list of known metrics sources\n\n\
         Options:\n  \
         -h --help             Show this help\n     \
         --version          Show package version\n     \
         --no-pager         Do not pipe output into a pager\n     \
         --no-legend        Do not show the headers and footers\n     \
         --user             Connect to user service manager\n     \
         --system           Connect to system service manager (default)\n     \
         --json=pretty|short\n                        Configure JSON output\n  \
         -j                    Equivalent to --json=pretty or --json=short"
    );
}

// ── Argument parsing ────────────────────────────────────────────────────────

enum ParseOutcome {
    Run(Options, Verb),
    ExitOk,
    ExitErr,
}

fn parse_argv(args: &[String]) -> ParseOutcome {
    let mut opts = Options {
        scope: Scope::System,
        legend: true,
        json: false,
        matches: Vec::new(),
    };
    let mut positionals: Vec<String> = Vec::new();
    let mut no_more = false;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if no_more || a == "-" || !a.starts_with('-') {
            positionals.push(a.clone());
            i += 1;
            continue;
        }
        if a == "--" {
            no_more = true;
            i += 1;
            continue;
        }
        if let Some(rest) = a.strip_prefix("--") {
            let (name, inline) = match rest.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (rest, None),
            };
            match name {
                "help" => {
                    help();
                    return ParseOutcome::ExitOk;
                }
                "version" => {
                    println!("systemd {} (systemd-report)", env!("CARGO_PKG_VERSION"));
                    return ParseOutcome::ExitOk;
                }
                "no-pager" => {}
                "no-legend" => opts.legend = false,
                "user" => opts.scope = Scope::User,
                "system" => opts.scope = Scope::System,
                "json" => {
                    let val = match inline {
                        Some(v) => v,
                        None => {
                            i += 1;
                            match args.get(i) {
                                Some(v) => v.clone(),
                                None => {
                                    eprintln!("--json= requires an argument");
                                    return ParseOutcome::ExitErr;
                                }
                            }
                        }
                    };
                    match val.as_str() {
                        "help" => {
                            println!("off\npretty\nshort");
                            return ParseOutcome::ExitOk;
                        }
                        "pretty" | "short" | "seq" | "auto" => opts.json = true,
                        "off" => opts.json = false,
                        other => {
                            eprintln!("Unknown --json= mode: {other}");
                            return ParseOutcome::ExitErr;
                        }
                    }
                }
                _ => {
                    eprintln!("Unknown option --{name}");
                    return ParseOutcome::ExitErr;
                }
            }
            i += 1;
            continue;
        }
        // Short options: -h, -j.
        for c in a[1..].chars() {
            match c {
                'h' => {
                    help();
                    return ParseOutcome::ExitOk;
                }
                'j' => opts.json = true,
                other => {
                    eprintln!("Unknown option -{other}");
                    return ParseOutcome::ExitErr;
                }
            }
        }
        i += 1;
    }

    // First positional is the verb; the rest are match args.
    let verb_str = positionals
        .first()
        .cloned()
        .unwrap_or_else(|| "help".to_string());
    let verb = match verb_str.as_str() {
        "help" => Verb::Help,
        "metrics" => Verb::Metrics,
        "describe-metrics" => Verb::DescribeMetrics,
        "list-sources" => Verb::ListSources,
        other => {
            eprintln!("Unknown command '{other}'.");
            return ParseOutcome::ExitErr;
        }
    };

    // Validate + collect match args (for metrics/describe-metrics).
    if matches!(verb, Verb::Metrics | Verb::DescribeMetrics) {
        for m in &positionals[1..] {
            if !metrics_name_valid(m) && !interface_name_is_valid(m) {
                eprintln!("Match is not a valid family name or prefix: {m}");
                return ParseOutcome::ExitErr;
            }
            opts.matches.push(m.clone());
        }
        opts.matches.sort();
        opts.matches.dedup();
    } else if positionals.len() > 1 && !matches!(verb, Verb::Help) {
        eprintln!("Too many arguments.");
        return ParseOutcome::ExitErr;
    }

    ParseOutcome::Run(opts, verb)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (opts, verb) = match parse_argv(&args) {
        ParseOutcome::Run(o, v) => (o, v),
        ParseOutcome::ExitOk => return ExitCode::SUCCESS,
        ParseOutcome::ExitErr => return ExitCode::from(1),
    };

    match verb {
        Verb::Help => {
            help();
            ExitCode::SUCCESS
        }
        Verb::Metrics => verb_metrics(&opts, false),
        Verb::DescribeMetrics => verb_metrics(&opts, true),
        Verb::ListSources => verb_list_sources(&opts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_names() {
        assert!(interface_name_is_valid("io"));
        assert!(interface_name_is_valid("piff"));
        assert!(interface_name_is_valid("io.systemd"));
        assert!(interface_name_is_valid("io.systemd.Network"));
        assert!(interface_name_is_valid("com.example.foo-bar"));
        assert!(!interface_name_is_valid("io."));
        assert!(!interface_name_is_valid(".io"));
        assert!(!interface_name_is_valid("io..systemd"));
        assert!(!interface_name_is_valid("1io"));
        assert!(!interface_name_is_valid(""));
    }

    #[test]
    fn field_names() {
        assert!(field_name_is_valid("receivePackets"));
        assert!(field_name_is_valid("a_b"));
        assert!(!field_name_is_valid("a__b"));
        assert!(!field_name_is_valid("a_"));
        assert!(!field_name_is_valid("1a"));
    }

    #[test]
    fn metric_names() {
        assert!(metrics_name_valid("io.systemd.Network.receivePackets"));
        assert!(!metrics_name_valid("piff")); // no dot -> not valid, but not an error
        assert!(!metrics_name_valid("io.")); // bad field
    }

    #[test]
    fn prefix_matching() {
        assert!(metric_startswith_prefix("io.systemd", "io"));
        assert!(metric_startswith_prefix(
            "io.systemd.Network.rx",
            "io.systemd.Network"
        ));
        assert!(!metric_startswith_prefix("io", "io"));
        assert!(!metric_startswith_prefix("ioctl", "io"));
    }

    #[test]
    fn match_args_all_accepted() {
        // The specific args from the test must all be accepted (not errors).
        for m in ["io", "io.systemd", "piff"] {
            assert!(
                metrics_name_valid(m) || interface_name_is_valid(m),
                "{m} rejected"
            );
        }
    }

    #[test]
    fn service_prefilter() {
        assert!(test_service_matches(&[], "io.systemd.Network"));
        assert!(test_service_matches(
            &["io".to_string()],
            "io.systemd.Network"
        ));
        assert!(test_service_matches(
            &["io.systemd.Network.rx".to_string()],
            "io.systemd.Network"
        ));
        assert!(!test_service_matches(
            &["com.example".to_string()],
            "io.systemd.Network"
        ));
    }
}

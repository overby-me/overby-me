//! Differential test (task #21): rust `systemd-analyze` vs the C
//! `systemd-analyze` for the deterministic time/calendar parsers.
//!
//! The CLI presentation differs (labels, timezone on the "Next elapse" line), so
//! this compares the SEMANTIC parse result, not raw stdout: the microsecond
//! value for `timespan`, and the "Normalized form:" line for `calendar` (which
//! uses an identical label in both and is timezone-independent). A divergence
//! there is real drift in the parser. Gated on env `SYSTEMD_ANALYZE` (path to
//! the C binary); skips silently otherwise. Run via `just differential`.

use std::process::Command;

fn run(bin: &str, args: &[&str]) -> (String, bool) {
    let out = Command::new(bin)
        .args(args)
        .env("TZ", "UTC")
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin} {args:?}: {e}"));
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

/// The "Normalized form:" line value (identical label in rust and C).
fn normalized_form(output: &str) -> Option<String> {
    output.lines().find_map(|l| {
        l.trim()
            .strip_prefix("Normalized form:")
            .map(|s| s.trim().to_string())
    })
}

/// The microsecond value from a `timespan` dump: rust prints "NNN us", C prints
/// "μs: NNN".
fn timespan_usec(output: &str) -> Option<String> {
    for line in output.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("μs:") {
            return Some(rest.trim().to_string());
        }
        if let Some(num) = t.strip_suffix(" us") {
            let n = num.trim();
            if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) {
                return Some(n.to_string());
            }
        }
    }
    None
}

#[test]
fn analyze_calendar_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");
    let specs = [
        "weekly",
        "daily",
        "monthly",
        "hourly",
        "yearly",
        "quarterly",
        "minutely",
        "Mon..Fri",
        "Sat,Sun 08:00",
        "*-*-* *:0/15:00",
        "*-*-1",
        "Mon *-*-* 00:00:00",
        "*:0/5",
        "12:00",
        "Mon,Tue *-1..7 12:00",
        "*-*-* 04:00:00",
        "Fri *-*-1..7 03:00:00",
    ];
    let mut div = Vec::new();
    for s in specs {
        let (ro, rok) = run(rust_bin, &["calendar", s]);
        let (co, cok) = run(&c_bin, &["calendar", s]);
        let rn = normalized_form(&ro);
        let cn = normalized_form(&co);
        if rn != cn || rok != cok {
            div.push(format!(
                "calendar {s:?}:\n     rust: ok={rok} norm={rn:?}\n     c   : ok={cok} norm={cn:?}"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze calendar drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

#[test]
fn analyze_timespan_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");
    let specs = [
        "1s",
        "2 days",
        "1h 30min",
        "1y",
        "500ms",
        "1us",
        "1000000",
        "10min",
        "1week 2days 3h",
        "5min 30s",
        "0",
        "100ms 200us",
        "3d",
        "1month",
        "2w",
    ];
    let mut div = Vec::new();
    for s in specs {
        let (ro, rok) = run(rust_bin, &["timespan", s]);
        let (co, cok) = run(&c_bin, &["timespan", s]);
        let ru = timespan_usec(&ro);
        let cu = timespan_usec(&co);
        if ru != cu || rok != cok {
            div.push(format!(
                "timespan {s:?}:\n     rust: ok={rok} usec={ru:?}\n     c   : ok={cok} usec={cu:?}"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze timespan drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

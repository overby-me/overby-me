//! Differential test (task #21): rust `systemd-analyze` vs the C
//! `systemd-analyze` for the deterministic time/calendar parsers.
//!
//! The CLI presentation differs (labels, timezone on the "Next elapse" line), so
//! this compares the SEMANTIC parse result, not raw stdout: the microsecond
//! value for `timespan`, and the "Normalized form:" line for `calendar` (which
//! uses an identical label in both and is timezone-independent). A divergence
//! there is real drift in the parser. Gated on env `SYSTEMD_ANALYZE` (path to
//! the C binary); skips silently otherwise. Run via `just differential`.

use std::collections::BTreeSet;
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

/// The "UNIX seconds:" line value (e.g. "@1704110400"), identical label in
/// rust and C. Timezone-independent, so a deterministic anchor for `timestamp`.
fn unix_seconds(output: &str) -> Option<String> {
    output.lines().find_map(|l| {
        l.trim()
            .strip_prefix("UNIX seconds:")
            .map(|s| s.trim().to_string())
    })
}

/// The (NAME, CLASS) pair from a single-status `exit-status` table: the row
/// after the header, whose first and last columns are the name and class (both
/// "-" for a known-numeric-but-unnamed status).
fn exit_status_name_class(output: &str) -> Option<(String, String)> {
    let row = output.lines().nth(1)?;
    let cols: Vec<&str> = row.split_whitespace().collect();
    Some((cols.first()?.to_string(), cols.last()?.to_string()))
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

#[test]
fn analyze_timestamp_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");
    // Absolute timestamps only: relative forms ("tomorrow", "+1h", "now") depend
    // on the wall clock and would race between the two binaries. These have a
    // fixed epoch, so `Normalized form:` and `UNIX seconds:` are deterministic.
    let specs = [
        "2024-01-01 12:00:00 UTC",
        "2024-06-15 08:30:00 UTC",
        "2024-01-01 UTC",
        "Mon 2024-01-01 12:00:00 UTC",
        // 2024-01-01 is a Monday, so this wrong weekday must be rejected by both.
        "Tue 2024-01-01 12:00:00 UTC",
        "2024-02-29 12:00:00 UTC",
        "2024-12-31 23:59:59 UTC",
        "1970-01-01 00:00:00 UTC",
        "2038-01-19 03:14:07 UTC",
        "2100-01-01 00:00:00 UTC",
        "2024-01-01 12:00:00.500000 UTC",
        "@0",
        "@1704110400",
        "@1000000000",
    ];
    let mut div = Vec::new();
    for s in specs {
        let (ro, rok) = run(rust_bin, &["timestamp", s]);
        let (co, cok) = run(&c_bin, &["timestamp", s]);
        let rn = normalized_form(&ro);
        let cn = normalized_form(&co);
        let ru = unix_seconds(&ro);
        let cu = unix_seconds(&co);
        if rn != cn || ru != cu || rok != cok {
            div.push(format!(
                "timestamp {s:?}:\n     rust: ok={rok} norm={rn:?} unix={ru:?}\n     c   : ok={cok} norm={cn:?} unix={cu:?}"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze timestamp drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

#[test]
fn analyze_exit_status_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");
    // Numeric statuses only: a non-numeric arg triggers C's strict error vs
    // rust's lenient warn-and-continue, an intentional behavioral difference.
    // The drift-prone data is the name/class table, so walk the ranges: libc,
    // LSB (1-8), BSD (64-78), systemd (200-243), plus unnamed gaps and 255.
    let mut specs: Vec<String> = Vec::new();
    for n in [0u32, 1, 2, 3, 4, 5, 6, 7, 8] {
        specs.push(n.to_string());
    }
    for n in 64..=78 {
        specs.push(n.to_string());
    }
    for n in 200..=243 {
        specs.push(n.to_string());
    }
    for n in [42u32, 100, 128, 199, 244, 250, 254, 255] {
        specs.push(n.to_string());
    }
    let mut div = Vec::new();
    for s in &specs {
        let (ro, rok) = run(rust_bin, &["exit-status", s]);
        let (co, cok) = run(&c_bin, &["exit-status", s]);
        let rn = exit_status_name_class(&ro);
        let cn = exit_status_name_class(&co);
        if rn != cn || rok != cok {
            div.push(format!(
                "exit-status {s}:\n     rust: ok={rok} {rn:?}\n     c   : ok={cok} {cn:?}"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze exit-status drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

#[test]
fn analyze_condition_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");
    // The two binaries format condition output very differently, so compare the
    // pass/fail verdict (exit 0 vs non-zero). Both read the same host, so a
    // host-state condition agrees unless the detection or parsing has drifted.
    // Kept host-independent by construction: any real kernel is >=1.0 and
    // <999.0; the probe paths are universal; architecture agrees on any host
    // since both read the same machine.
    let specs = [
        "ConditionKernelVersion=>=1.0",
        "ConditionKernelVersion=<1.0",
        "ConditionKernelVersion=>=999.0",
        "ConditionKernelVersion=!=999.99",
        "ConditionKernelVersion=<999.0",
        "ConditionArchitecture=x86-64",
        "ConditionArchitecture=arm64",
        "ConditionPathExists=/proc",
        "ConditionPathExists=/nonexistent-differential-xyz",
        "ConditionPathExists=!/nonexistent-differential-xyz",
        "ConditionPathIsDirectory=/proc",
        "ConditionPathIsDirectory=/nonexistent-differential-xyz",
        // Regression: FirstBoot was an unhandled condition in rust `analyze`.
        "ConditionFirstBoot=no",
        "ConditionFirstBoot=yes",
        "AssertPathExists=/proc",
        // Regression: these were "Unknown condition type" in rust `analyze`
        // until it delegated to libsystemd's evaluator. Robust by construction:
        // any host has >=1 CPU and <1024000T RAM; the os-release value cannot
        // match a real distro; the rest read the same host in both binaries.
        "ConditionCPUs=>=1",
        "ConditionCPUs=<1",
        "ConditionMemory=>=1",
        "ConditionMemory=>=1024000T",
        "ConditionOSRelease=ID=zzz-not-a-real-distro-999",
        "ConditionControlGroupController=cpu",
        "ConditionUser=root",
        "ConditionUser=@system",
        "ConditionGroup=0",
        "ConditionSecurity=selinux",
        "ConditionKernelCommandLine=quiet",
    ];
    let mut div = Vec::new();
    for s in specs {
        let rok = run(rust_bin, &["condition", s]).1;
        let cok = run(&c_bin, &["condition", s]).1;
        if rok != cok {
            div.push(format!("condition {s:?}: rust_pass={rok} c_pass={cok}"));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze condition drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

#[test]
fn analyze_capability_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");
    // The Linux capability name<->number table gains entries between kernel and
    // systemd releases; compare the full set of (NAME, NUMBER) pairs.
    let pairs = |bin: &str| -> BTreeSet<(String, String)> {
        run(bin, &["capability"])
            .0
            .lines()
            .skip(1) // header: "NAME  NUMBER"
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                Some((it.next()?.to_string(), it.next()?.to_string()))
            })
            .collect()
    };
    let c = pairs(&c_bin);
    let r = pairs(rust_bin);
    assert!(!c.is_empty(), "C systemd-analyze capability produced no rows");

    let only_c: Vec<_> = c.difference(&r).collect();
    let only_r: Vec<_> = r.difference(&c).collect();
    assert!(
        only_c.is_empty() && only_r.is_empty(),
        "rust vs C systemd-analyze capability drift:\n  only in C ({}): {:?}\n  only in rust ({}): {:?}",
        only_c.len(),
        only_c,
        only_r.len(),
        only_r
    );
}

/// `compare-versions` implements systemd's strverscmp_improved. Both the printed
/// relation (stdout) and the exit-status encoding (success: exit 0 for `==`,
/// non-zero for `<`/`>`, and for the operator form whether the comparison holds)
/// must match C. A divergence is real drift in the version-ordering algorithm.
#[test]
fn compare_versions_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");

    // Pairs exercising '~' pre-releases, '-'/'^'/'.' separators, numeric-vs-alpha
    // segments, leading zeros, and the documented ascending sort order.
    let cases: &[&[&str]] = &[
        &["compare-versions", "1", "2"],
        &["compare-versions", "2", "1"],
        &["compare-versions", "2.0", "2.0"],
        &["compare-versions", "1.0-1", "1.0-2"],
        &["compare-versions", "1~rc1", "1"],
        &["compare-versions", "4.5~alpha1", "4.5"],
        &["compare-versions", "007", "7"],
        &["compare-versions", "122.1", "123~rc1-1"],
        &["compare-versions", "123~rc1-1", "123"],
        &["compare-versions", "123", "123-a"],
        &["compare-versions", "123-1", "123-1.1"],
        &["compare-versions", "123-1.1", "123^post1"],
        &["compare-versions", "123^post1", "123.a-1"],
        &["compare-versions", "123.1-1", "123a-1"],
        &["compare-versions", "123a-1", "124-1"],
        &["compare-versions", "5.11.0", "5.11.1"],
        &["compare-versions", "6.1", "6.1.0"],
        // Operator form: no stdout, exit reflects whether the comparison holds.
        &["compare-versions", "1", "lt", "2"],
        &["compare-versions", "2", "lt", "1"],
        &["compare-versions", "1~rc1", "lt", "1"],
        &["compare-versions", "5", "ge", "5"],
    ];

    let mut divergences = Vec::new();
    for args in cases {
        let (rust_out, rust_ok) = run(rust_bin, args);
        let (c_out, c_ok) = run(&c_bin, args);
        if rust_out != c_out || rust_ok != c_ok {
            divergences.push(format!(
                "args={args:?}\n     rust: ok={rust_ok} out={rust_out:?}\n     c   : ok={c_ok} out={c_out:?}"
            ));
        }
    }
    assert!(
        divergences.is_empty(),
        "rust vs C systemd-analyze compare-versions drift ({} case(s)):\n{}",
        divergences.len(),
        divergences.join("\n")
    );
}

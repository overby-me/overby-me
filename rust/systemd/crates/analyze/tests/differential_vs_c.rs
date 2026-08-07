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
        // Day-of-month counted from the end of the month (`~`).
        "*-*~01",
        "*-*~2",
        "*-02~01",
        "*-*~1..3",
        "*-*~1,15",
        "Mon *-*~07/1",
        "*-*~28",
        // Rejected forms: a second `~`, a reversed from-end range, and a
        // reversed ordinary range (upstream errors on all three).
        "*-*~5..~1",
        "*-*~7..1",
        "*-*-7..5",
        "*:30..10",
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
    // Compare exact stdout, stderr, and exit for every status 0..=255 (locking
    // the whole name/class table, the auto-sized columns, and the "-"/"-" form
    // for unnamed-but-valid codes), plus behavioral edge cases: the full table,
    // argument order (no sort), the exact "Invalid exit status" error printed
    // before any table, case-sensitive name matching, and a partial list that
    // must error without printing the valid rows before it.
    let mut cases: Vec<Vec<String>> = Vec::new();
    for n in 0u32..=255 {
        cases.push(vec!["exit-status".to_string(), n.to_string()]);
    }
    for extra in [
        vec!["exit-status"],
        vec!["exit-status", "1", "0"],
        vec!["exit-status", "0", "1", "246"],
        vec!["exit-status", "MEMORY_THP"],
        vec!["exit-status", "STDOUT"],
        vec!["exit-status", "EXCEPTION"],
        vec!["exit-status", "SUCCESS", "FAILURE", "MEMORY_THP"],
        vec!["exit-status", "256"],
        vec!["exit-status", "FOOBAR"],
        vec!["exit-status", "success"],
        vec!["exit-status", "SUCCESS", "FOOBAR"],
    ] {
        cases.push(extra.into_iter().map(String::from).collect());
    }

    let mut div = Vec::new();
    for case in &cases {
        let args: Vec<&str> = case.iter().map(String::as_str).collect();
        let (ro, re, rok) = run_full(rust_bin, &args);
        let (co, ce, cok) = run_full(&c_bin, &args);
        if ro != co || re != ce || rok != cok {
            div.push(format!(
                "args={case:?}\n     rust: ok={rok} out={ro:?} err={re:?}\n     c   : ok={cok} out={co:?} err={ce:?}"
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

/// Run `bin args...` and capture (stdout, stderr, success) verbatim. Used by the
/// capability oracle, which must lock the error text (stderr) too.
fn run_full(bin: &str, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(bin)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {bin} {args:?}: {e}"));
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

/// Companion to `analyze_capability_matches_c` (which compares the full table's
/// (NAME, NUMBER) pair *set*, ignoring order and whitespace). This one locks the
/// exact output *behavior* the set comparison cannot see: stdout, stderr, and
/// exit status are compared verbatim against C for a corpus of explicit lookups
/// and error cases. The corpus is deliberately kernel-independent: the
/// unqualified full table and mid-range numbers depend on the host's
/// cap_last_cap() (C shows caps up to MAX(CAP_LAST_CAP, cap_last_cap())), so it
/// only names capabilities <= 40 (present on any kernel since 5.9), plus bogus
/// names and 999 (beyond the capability model, always unknown). It locks the
/// number-sorted ordering, C's column widths, case-insensitive full-name
/// matching, the rejection of a bare name without the "cap_" prefix, and the
/// print-nothing-on-error behavior with C's exact "is not known." message.
#[test]
fn analyze_capability_output_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");

    let cases: &[&[&str]] = &[
        &["capability", "cap_sys_admin"],
        &["capability", "21"],
        &["capability", "cap_sys_admin", "cap_chown"], // sorted by number
        &["capability", "0", "40"],
        &["capability", "CAP_SYS_ADMIN"], // case-insensitive
        &["capability", "Cap_Sys_Admin"],
        &["capability", "cap_SYS_admin"],
        &["capability", "cap_checkpoint_restore", "cap_chown", "cap_bpf"], // sort + width
        &["capability", "sys_admin"], // bare name (no cap_ prefix) is rejected
        &["capability", "SYS_ADMIN"],
        &["capability", "cap_bogus_nonexistent"],
        &["capability", "999"], // numeric, beyond the capability model
        &["capability", "cap_chown", "cap_bogus"], // partial list -> no output, error
        // Mask mode: -m/--mask is a flag, the positional is a hex mask.
        &["capability", "--mask", "0x3"], // bits 0,1
        &["capability", "--mask", "3c00"], // bits 10-13, no 0x prefix
        &["capability", "-m", "0000000000003c00"], // leading zeros, short flag
        &["capability", "--mask", "0"], // no bits set -> header only
        &["capability", "--mask"], // missing positional
        &["capability", "--mask", "0x3", "0x4"], // too many positionals
        &["capability", "--mask", "zzz"], // unparseable mask
    ];

    let mut divergences = Vec::new();
    for args in cases {
        let (r_out, r_err, r_ok) = run_full(rust_bin, args);
        let (c_out, c_err, c_ok) = run_full(&c_bin, args);
        if r_out != c_out || r_err != c_err || r_ok != c_ok {
            divergences.push(format!(
                "args={args:?}\n     rust: ok={r_ok} out={r_out:?} err={r_err:?}\n     c   : ok={c_ok} out={c_out:?} err={c_err:?}"
            ));
        }
    }
    assert!(
        divergences.is_empty(),
        "rust vs C systemd-analyze capability drift ({} case(s)):\n{}",
        divergences.len(),
        divergences.join("\n")
    );
}

/// The calendar output's deterministic lines must match C: the "Original form"
/// line appears only when it differs from the normalized form, and each "Next
/// elapse"/"Iteration #N" timestamp (computed from a fixed --base-time) matches.
/// The "From now" line is relative to the real clock, so it is filtered out.
#[test]
fn analyze_calendar_form_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");

    // Deterministic lines only: drop "From now" (real-clock relative) and blanks.
    let det = |bin: &str, args: &[&str]| -> String {
        run(bin, args)
            .0
            .lines()
            .filter(|l| !l.trim_start().starts_with("From now") && !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    };

    // All cases pin --base-time so the elapse timestamps are deterministic
    // (without it, "Next elapse" is relative to the current clock).
    let base = "--base-time=2026-01-01 00:00:00 UTC";
    let cases: &[&[&str]] = &[
        &["calendar", base, "*-*-* 06:00:00"], // already normalized -> no Original form
        &["calendar", base, "monday"],         // -> "Original form: monday"
        &["calendar", base, "12:00"],
        &["calendar", base, "--iterations=3", "Mon *-*-* 00:00:00"],
        &["calendar", base, "--iterations=5", "*-*-* 06:00:00"],
        &["calendar", base, "--iterations=2", "Sat,Sun 08:00:00"],
    ];

    let mut div = Vec::new();
    for args in cases {
        let (r, c) = (det(rust_bin, args), det(&c_bin, args));
        if r != c {
            div.push(format!("args={args:?}\n  C:\n{c}\n  R:\n{r}"));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze calendar form drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

/// `systemd-analyze timespan` output is fully deterministic (no wall clock), so
/// compare it verbatim against C: the "Original"/"μs"/"Human" vertical table
/// with the microsecond-precise human form ("1.500000s", "1.000001s") and a
/// blank line only between multiple inputs.
#[test]
fn analyze_timespan_output_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");

    let single = [
        "1s", "2s", "1s 500ms", "1s 1us", "1.5s", "90s", "3661s", "1min", "61s",
        "100ms", "500ms", "1us", "0", "infinity", "1y", "2w 3d", "1h 30min",
        "999999", "1000000", "1000001", "59999999", "60000000", "1month",
    ];
    let mut cases: Vec<Vec<&str>> = single.iter().map(|s| vec!["timespan", s]).collect();
    // Multiple inputs are separated by a blank line (none after the last).
    cases.push(vec!["timespan", "1s", "2s", "3us"]);

    let mut div = Vec::new();
    for args in &cases {
        let (ro, rok) = run(rust_bin, args);
        let (co, cok) = run(&c_bin, args);
        if ro != co || rok != cok {
            div.push(format!(
                "args={args:?}\n  C:\n{co}\n  R:\n{ro}"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze timespan output drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

/// `systemd-analyze timestamp` output, minus the wall-clock-relative "From now"
/// line, is deterministic under a fixed TZ (the `run` helper pins TZ=UTC). Lock
/// it against C: the Original/Normalized/UNIX-seconds lines, the "(in UTC)" line
/// omitted when the rendering is already UTC, and a blank only between inputs.
#[test]
fn analyze_timestamp_output_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");

    // Drop the "From now" line (relative to the real clock).
    let det = |bin: &str, args: &[&str]| -> String {
        run(bin, args)
            .0
            .lines()
            .filter(|l| !l.trim_start().starts_with("From now"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let single = [
        "2024-01-01 12:00:00 UTC",
        "@0",
        "@1704110400",
        "2038-01-19 03:14:07 UTC",
        "1970-01-01 00:00:00 UTC",
        "2024-02-29 12:00:00 UTC",
        "2024-01-01 12:00:00.500000 UTC",
        "Mon 2024-01-01 12:00:00 UTC",
        "2100-01-01 00:00:00 UTC",
    ];
    let mut cases: Vec<Vec<&str>> = single.iter().map(|s| vec!["timestamp", s]).collect();
    cases.push(vec!["timestamp", "@0", "@1704110400"]); // blank between inputs

    let mut div = Vec::new();
    for args in &cases {
        let (r, c) = (det(rust_bin, args), det(&c_bin, args));
        if r != c {
            div.push(format!("args={args:?}\n  C:\n{c}\n  R:\n{r}"));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze timestamp output drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

/// `systemd-analyze architectures` lists the known CPU architectures. The name
/// set and enum ordering are static; the SUPPORT column (native/secondary/uname)
/// depends on the running architecture, but rust and C are built for the same
/// host here, so a verbatim stdout+stderr+exit comparison holds. Covers the full
/// table, explicit lists (sorted by enum id), the native/uname/secondary
/// keywords, and the "not known" error before any table.
#[test]
fn analyze_architectures_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");

    let cases: &[&[&str]] = &[
        &["architectures"],
        &["architectures", "x86-64", "arm64"],
        &["architectures", "native"],
        &["architectures", "uname"],
        &["architectures", "secondary"],
        &["architectures", "loongarch64", "riscv64", "s390x"],
        &["architectures", "x86", "x86-64"],
        &["architectures", "bogus"],
        &["architectures", "x86-64", "bogus", "arm64"],
    ];

    let mut div = Vec::new();
    for args in cases {
        let (ro, re, rok) = run_full(rust_bin, args);
        let (co, ce, cok) = run_full(&c_bin, args);
        if ro != co || re != ce || rok != cok {
            div.push(format!(
                "args={args:?}\n     rust: ok={rok} out={ro:?} err={re:?}\n     c   : ok={cok} out={co:?} err={ce:?}"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze architectures drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

/// `systemd-analyze filesystems` dumps the predefined filesystem sets (@basic-api
/// etc.) with each member's statfs magic(s) and "[owner]" aliases. The set data
/// is static; the no-arg listing's trailing "Unlisted" section reads
/// /proc/filesystems, but rust and C read the same host, so a verbatim
/// stdout/stderr/exit comparison holds. Covers every set, the full listing, a
/// multi-set request, and the "not found" error.
#[test]
fn analyze_filesystems_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");

    let cases: &[&[&str]] = &[
        &["filesystems"], // full listing incl. Ungrouped/Unlisted sections
        &["filesystems", "@basic-api"],
        &["filesystems", "@network"], // multi-magic + [owner] aliases
        &["filesystems", "@known"],
        &["filesystems", "@anonymous"],
        &["filesystems", "@application"],
        &["filesystems", "@auxiliary-api"],
        &["filesystems", "@common-block"],
        &["filesystems", "@historical-block"],
        &["filesystems", "@privileged-api"],
        &["filesystems", "@security"],
        &["filesystems", "@temporary"],
        &["filesystems", "@basic-api", "@network"], // blank line between
        &["filesystems", "bogus"],
        &["filesystems", "@bogus"],
    ];

    let mut div = Vec::new();
    for args in cases {
        let (ro, re, rok) = run_full(rust_bin, args);
        let (co, ce, cok) = run_full(&c_bin, args);
        if ro != co || re != ce || rok != cok {
            div.push(format!(
                "args={args:?}\n     rust: ok={rok} err={re:?}\n     c   : ok={cok} err={ce:?}\n     (stdout diff omitted)"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze filesystems drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

/// `systemd-analyze syscall-filter` dumps the predefined seccomp syscall sets
/// (@default, @system-service, ...) with nested "@" references printed as-is. The
/// set data is static; the no-arg listing's "Unlisted" section / notice reads
/// tracefs, but rust and C read the same host, so a verbatim stdout/stderr/exit
/// comparison holds. Covers several sets (incl. nested refs), the full listing,
/// and the "not found" error.
#[test]
fn analyze_syscall_filter_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");

    let cases: &[&[&str]] = &[
        &["syscall-filter"], // full listing incl. Ungrouped + Unlisted/notice
        &["syscall-filter", "@default"], // has a nested @sandbox ref
        &["syscall-filter", "@basic-io"],
        &["syscall-filter", "@known"], // has a nested @obsolete ref
        &["syscall-filter", "@system-service"], // 16 nested refs
        &["syscall-filter", "@privileged"],
        &["syscall-filter", "@obsolete"],
        &["syscall-filter", "@sandbox"],
        &["syscall-filter", "@basic-io", "@aio"], // blank line between
        &["syscall-filter", "bogus"],
        &["syscall-filter", "@bogus"],
    ];

    let mut div = Vec::new();
    for args in cases {
        let (ro, re, rok) = run_full(rust_bin, args);
        let (co, ce, cok) = run_full(&c_bin, args);
        if ro != co || re != ce || rok != cok {
            div.push(format!(
                "args={args:?}\n     rust: ok={rok} err={re:?}\n     c   : ok={cok} err={ce:?}\n     (stdout diff omitted)"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze syscall-filter drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

/// `systemd-analyze transient-settings <unittype>` lists the properties settable
/// on a transient unit of that type, one per line, in C's bus-property-table
/// order (a static per-type list). Compares stdout/stderr/exit verbatim for every
/// unit type, multiple types (blank line between), and the error cases (no
/// argument, invalid type). Streams are captured separately, so C's stdio
/// interleaving of a mid-run error does not matter.
#[test]
fn analyze_transient_settings_matches_c() {
    let Ok(c_bin) = std::env::var("SYSTEMD_ANALYZE") else {
        eprintln!("skip differential: SYSTEMD_ANALYZE unset (run `just differential`)");
        return;
    };
    let rust_bin = env!("CARGO_BIN_EXE_systemd-analyze");

    let cases: &[&[&str]] = &[
        &["transient-settings", "service"],
        &["transient-settings", "socket"],
        &["transient-settings", "target"],
        &["transient-settings", "device"],
        &["transient-settings", "mount"],
        &["transient-settings", "automount"],
        &["transient-settings", "timer"],
        &["transient-settings", "swap"],
        &["transient-settings", "path"],
        &["transient-settings", "slice"],
        &["transient-settings", "scope"],
        &["transient-settings", "service", "mount"], // blank line between
        &["transient-settings"],                     // Too few arguments.
        &["transient-settings", "bogus"],            // Invalid unit type 'bogus'.
        &["transient-settings", "service", "bogus", "mount"], // errors after service
    ];

    let mut div = Vec::new();
    for args in cases {
        let (ro, re, rok) = run_full(rust_bin, args);
        let (co, ce, cok) = run_full(&c_bin, args);
        if ro != co || re != ce || rok != cok {
            div.push(format!(
                "args={args:?}\n     rust: ok={rok} err={re:?}\n     c   : ok={cok} err={ce:?}\n     (stdout diff omitted)"
            ));
        }
    }
    assert!(
        div.is_empty(),
        "rust vs C systemd-analyze transient-settings drift ({}):\n{}",
        div.len(),
        div.join("\n")
    );
}

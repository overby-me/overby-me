// pw-profiler: collect profiler events.

use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args
        .first()
        .map(String::as_str)
        .unwrap_or("pw-profiler");
    let args = expand_short_clusters(raw_args, &['h', 'V', 'J']);
    let required_args: &[(&str, &str, char)] = &[
        ("-r", "--remote", 'r'),
        ("-o", "--output", 'o'),
        ("-n", "--iterations", 'n'),
    ];
    let mut i = 1;
    while i < args.len() {
        let s = args[i].as_str();
        match s {
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "--version" | "-V" => {
                print_version(argv0);
                return 0;
            }
            s if s.starts_with("--help=") => {
                eprintln!("{argv0}: option '--help' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            "-J" | "--json" => {}
            // Required-arg flags (long-form errors say "option '--FLAG'").
            s if required_args
                .iter()
                .any(|(short, long, _)| *short == s || *long == s) =>
            {
                let (_, long, ch) = required_args
                    .iter()
                    .find(|(short, long, _)| *short == s || *long == s)
                    .copied()
                    .unwrap();
                if i + 1 >= args.len() {
                    if s.starts_with("--") {
                        eprintln!("{argv0}: option '{long}' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- '{ch}'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
                i += 2;
                continue;
            }
            // Inline `--FOO=value`.
            s if s.starts_with("--") && s.contains('=') => {
                let name = s.split_once('=').map(|(n, _)| n).unwrap_or(s);
                if required_args.iter().any(|(_, l, _)| *l == name) {
                    i += 1;
                    continue;
                }
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            // Short attached `-r<value>`, `-o<value>` etc.
            s if s.starts_with('-') && !s.starts_with("--") && s.len() > 2 => {
                let short = &s[..2];
                if required_args.iter().any(|(sh, _, _)| *sh == short) {
                    i += 1;
                    continue;
                }
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            _ => {
                // Positional; pw-profiler doesn't use them but C accepts.
                break;
            }
        }
        i += 1;
    }
    // C connects to the daemon to collect profiler events. Try
    // connecting; emit C's "Can't connect" error if it fails
    // (capital C, matches pw-profiler.c's show_error).
    let env_remote = std::env::var("PIPEWIRE_REMOTE")
        .ok()
        .filter(|s| !s.is_empty());
    let connect = if let Some(name) = &env_remote {
        let runtime = std::env::var("PIPEWIRE_RUNTIME_DIR")
            .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let path = std::path::PathBuf::from(runtime).join(name);
        crate::pipewire_lib::client::Client::connect_path(&path)
    } else {
        crate::pipewire_lib::client::Client::connect_default()
    };
    match connect {
        Ok(_) => 0,
        Err(_) => {
            eprintln!(
                "Can't connect: {}",
                crate::tools::common::connect_failure_msg_for(env_remote.as_deref())
            );
            u8::MAX as i32
        }
    }
}

fn print_help(argv0: &str) {
    println!("{argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -r, --remote                          Remote daemon name");
    println!(
        "  -o, --output                          Profiler output name (default \"profiler.log\")"
    );
    println!("  -J, --json                            Dump raw data as JSON");
    println!("  -n, --iterations                      Collect this many samples");
}

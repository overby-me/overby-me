// pw-loopback: create a loopback node pair. Real implementation needs
// the pw_stream API (Phase 8). Here we just emit upstream's --help with
// the current PID baked into the default node-name.

use crate::tools::common::print_version;
use std::process;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-loopback");

    // All long required-arg options have a matching short letter; this
    // table maps short → (long, char) for error messages.
    let required_args: &[(&str, &str, char)] = &[
        ("-r", "--remote", 'r'),
        ("-n", "--name", 'n'),
        ("-g", "--group", 'g'),
        ("-c", "--channels", 'c'),
        ("-m", "--channel-map", 'm'),
        ("-l", "--latency", 'l'),
        ("-d", "--delay", 'd'),
        ("-C", "--capture", 'C'),
        ("-i", "--capture-props", 'i'),
        ("-P", "--playback", 'P'),
        ("-o", "--playback-props", 'o'),
    ];
    let mut explicit_remote: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
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
            // Inline `--FOO=value` for any required-arg long flag.
            s if s.starts_with("--") && s.contains('=') => {
                let name = s.split_once('=').map(|(n, _)| n).unwrap_or(s);
                if required_args.iter().any(|(_, l, _)| *l == name) {
                    if name == "--remote" {
                        explicit_remote = Some(s["--remote=".len()..].to_string());
                    }
                    i += 1;
                    continue;
                }
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            // Bare required-arg flag: must consume a following value.
            s if required_args.iter().any(|(short, long, _)| *short == s || *long == s) => {
                let (_, long, ch) = required_args.iter()
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
                if s == "-r" || s == "--remote" {
                    explicit_remote = Some(args[i + 1].clone());
                }
                i += 2;
                continue;
            }
            s if s.starts_with('-') && !s.starts_with("--") && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            // Short attached-value form `-r<value>` etc.
            s if s.starts_with('-') && !s.starts_with("--") && s.len() > 2 => {
                let short = &s[..2];
                if required_args.iter().any(|(sh, _, _)| *sh == short) {
                    if short == "-r" {
                        explicit_remote = Some(s[2..].to_string());
                    }
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
            _ => {}
        }
        i += 1;
    }

    // pw-loopback always tries to load the module after option parsing.
    // We don't have module loading yet, so emit the same error C does
    // on failure. (The C tool also errors when the daemon is up but the
    // module path doesn't include libpipewire-module-loopback in our
    // sandbox, hence the consistent error in both cases.)
    let env_remote = std::env::var("PIPEWIRE_REMOTE").ok().filter(|s| !s.is_empty());
    let chosen: Option<String> = explicit_remote.or(env_remote);
    eprintln!(
        "can't load module: {}",
        crate::tools::common::connect_failure_msg_for(chosen.as_deref())
    );
    u8::MAX as i32
}

fn print_help(argv0: &str) {
    let pid = process::id();
    println!("{argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -r, --remote                          Remote daemon name");
    // C uses pw_get_application_name() which is argv[0] basename without
    // the path — but with `-<pid>` appended. With full path argv[0],
    // C's `pw-loopback-<pid>` becomes `<full argv[0]>-<pid>` because
    // the application name function fell back to the full string.
    println!("  -n, --name                            Node name (default '{argv0}-{pid}')");
    println!("  -g, --group                           Node group (default '{argv0}-{pid}')");
    println!("  -c, --channels                        Number of channels (default 2)");
    println!("  -m, --channel-map                     Channel map (default '[ FL, FR ]')");
    println!("  -l, --latency                         Desired latency in ms");
    println!("  -d, --delay                           Desired delay in float s");
    println!(
        "  -C  --capture                         Capture source to connect to (name or serial)"
    );
    println!("  -i  --capture-props                   Capture stream properties");
    println!(
        "  -P  --playback                        Playback sink to connect to (name or serial)"
    );
    println!("  -o  --playback-props                  Playback stream properties");
}

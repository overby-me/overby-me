// pipewire / pipewire-pulse: the daemon. Phase 0 only handles
// --help and --version; the actual server is Phase 9.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pipewire");
    // Each daemon variant has its own default config name. We infer it
    // from argv0 — the C tools all derive their default the same way.
    let default_config = if argv0.contains("pulse") {
        "pipewire-pulse.conf"
    } else if argv0.contains("aes67") {
        "pipewire-aes67.conf"
    } else if argv0.contains("avb") {
        "pipewire-avb.conf"
    } else if argv0.contains("vulkan") {
        "pipewire-vulkan.conf"
    } else {
        "pipewire.conf"
    };

    match args.get(1).map(String::as_str) {
        Some("-h") | Some("--help") => {
            print_help(argv0, default_config);
            0
        }
        Some("--version") | Some("-V") => {
            print_version(argv0);
            0
        }
        Some(s) if s.starts_with("--help=") => {
            eprintln!("{argv0}: option '--help' doesn't allow an argument");
            234
        }
        Some(s) if s.starts_with("--version=") => {
            eprintln!("{argv0}: option '--version' doesn't allow an argument");
            234
        }
        // -v / --verbose are no-arg.
        Some("-v") | Some("--verbose") => {
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
        // -c / --config / -P / --properties REQUIRE a value. With no
        // following arg getopt errors. C maps the -EINVAL fallthrough to
        // exit 234 (= -22 truncated).
        Some(opt @ ("-c" | "--config")) => {
            if args.len() > 2 {
                // Has value but daemon not yet implemented.
                eprintln!("{argv0}: not yet implemented in rust-pipewire");
                1
            } else if opt == "--config" {
                eprintln!("{argv0}: option '--config' requires an argument");
                234
            } else {
                eprintln!("{argv0}: option requires an argument -- 'c'");
                234
            }
        }
        Some(opt @ ("-P" | "--properties")) => {
            if args.len() > 2 {
                eprintln!("{argv0}: not yet implemented in rust-pipewire");
                1
            } else if opt == "--properties" {
                eprintln!("{argv0}: option '--properties' requires an argument");
                234
            } else {
                eprintln!("{argv0}: option requires an argument -- 'P'");
                234
            }
        }
        // Inline-value forms — accepted, daemon not yet implemented.
        Some(s) if s.starts_with("--config=") || s.starts_with("--properties=") => {
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
        Some(s) if s.starts_with("--") => {
            eprintln!("{argv0}: unrecognized option '{s}'");
            234
        }
        Some(s) if s.starts_with("-c") && s.len() > 2 => {
            // -cfoo (attached value)
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
        Some(s) if s.starts_with("-P") && s.len() > 2 => {
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
        Some(s) if s.starts_with('-') && s.len() == 2 => {
            let ch = s.chars().nth(1).unwrap_or('?');
            eprintln!("{argv0}: invalid option -- '{ch}'");
            234
        }
        Some(s) if s.starts_with('-') => {
            eprintln!("{argv0}: unrecognized option '{s}'");
            234
        }
        _ => {
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
    }
}

fn print_help(argv0: &str, default_config: &str) {
    println!("{argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("  -v, --verbose                         Increase verbosity by one level");
    println!("      --version                         Show version");
    println!("  -c, --config                          Load config (Default {default_config})");
    println!("  -P  --properties                      Set context properties");
}

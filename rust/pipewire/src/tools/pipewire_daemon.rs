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
        // Recognized flags we don't yet implement — running the daemon
        // is Phase 9. Just print "not implemented" so callers know.
        Some("-v") | Some("--verbose") | Some("-c") | Some("--config") | Some("-P")
        | Some("--properties") => {
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
        Some(s) if s.starts_with('-') => {
            // getopt_long writes the unrecognized-option message; C main
            // returns res = -EINVAL (-22) which truncates to 234.
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

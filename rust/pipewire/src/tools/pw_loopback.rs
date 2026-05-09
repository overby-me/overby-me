// pw-loopback: create a loopback node pair. Real implementation needs
// the pw_stream API (Phase 8). Here we just emit upstream's --help with
// the current PID baked into the default node-name.

use crate::tools::common::print_version;
use std::process;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-loopback");

    for a in args.iter().skip(1) {
        match a.as_str() {
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
                return 0;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with('-')
                && !matches!(
                    s,
                    "-r" | "--remote"
                        | "-n"
                        | "--name"
                        | "-g"
                        | "--group"
                        | "-c"
                        | "--channels"
                        | "-m"
                        | "--channel-map"
                        | "-l"
                        | "--latency"
                        | "-d"
                        | "--delay"
                        | "-C"
                        | "--capture"
                        | "-i"
                        | "--capture-props"
                        | "-P"
                        | "--playback"
                        | "-o"
                        | "--playback-props"
                ) =>
            {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            _ => {}
        }
    }

    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
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

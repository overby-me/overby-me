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
            // Recognized flags (all required-arg per pw-loopback's
            // long_options): match the bare form or the `--FOO=value`
            // inline form.
            s if s == "-r"
                || s == "--remote"
                || s == "-n"
                || s == "--name"
                || s == "-g"
                || s == "--group"
                || s == "-c"
                || s == "--channels"
                || s == "-m"
                || s == "--channel-map"
                || s == "-l"
                || s == "--latency"
                || s == "-d"
                || s == "--delay"
                || s == "-C"
                || s == "--capture"
                || s == "-i"
                || s == "--capture-props"
                || s == "-P"
                || s == "--playback"
                || s == "-o"
                || s == "--playback-props"
                || s.starts_with("--remote=")
                || s.starts_with("--name=")
                || s.starts_with("--group=")
                || s.starts_with("--channels=")
                || s.starts_with("--channel-map=")
                || s.starts_with("--latency=")
                || s.starts_with("--delay=")
                || s.starts_with("--capture=")
                || s.starts_with("--capture-props=")
                || s.starts_with("--playback=")
                || s.starts_with("--playback-props=") => {}
            s if s.starts_with('-') => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            _ => {}
        }
    }

    // pw-loopback always tries to load the module after option parsing.
    // We don't have module loading yet, so emit the same error C does
    // on failure. (The C tool also errors when the daemon is up but the
    // module path doesn't include libpipewire-module-loopback in our
    // sandbox, hence the consistent error in both cases.)
    eprintln!(
        "can't load module: {}",
        crate::tools::common::connect_failure_msg()
    );
    0
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

// pw-link: PipeWire port and link manager.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-link");
    match args.get(1).map(String::as_str) {
        Some("-h") | Some("--help") | None => {
            print_help(argv0);
            0
        }
        Some("--version") => {
            print_version(argv0);
            0
        }
        _ => {
            eprintln!("{argv0}: not yet implemented in rust-pipewire");
            1
        }
    }
}

fn print_help(argv0: &str) {
    println!("{argv0} : PipeWire port and link manager.");
    println!("Generic: {argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -r, --remote=NAME                     Remote daemon name");
    println!("List: {argv0} [options] [out-pattern] [in-pattern]");
    println!("  -o, --output                          List output ports");
    println!("  -i, --input                           List input ports");
    println!("  -l, --links                           List links");
    println!("  -t, --latency                         List port latencies");
    println!("  -m, --monitor                         Monitor links and ports");
    println!("  -I, --id                              List IDs");
    println!("  -v, --verbose                         Verbose port properties");
    println!("Connect: {argv0} [options] output input");
    println!("  -L, --linger                          Linger (default, unless -m is used)");
    println!("  -P, --passive                         Passive link");
    println!("  -p, --props=PROPS                     Properties as JSON object");
    println!("  -w, --wait                            Wait until link creation attempt");
    println!("Disconnect: {argv0} -d [options] output input");
    println!("            {argv0} -d [options] link-id");
    println!("  -d, --disconnect                      Disconnect ports");
}

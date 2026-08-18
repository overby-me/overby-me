// pw-v4l2: a setuid wrapper that LD_PRELOADs the PipeWire v4l2
// implementation before exec'ing a v4l2 client. The C tool is a tiny
// shell-style getopt wrapper; we mirror its -h output verbatim.

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-v4l2");

    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        // pw-v4l2's optstring is `+hr:v` (POSIXly correct, requires r-arg).
        // Old-style BSD getopt: `+` makes errors `illegal option`.
        if !a.starts_with('-') || a == "-" || a == "--" {
            // First positional ends option scanning.
            break;
        }
        // Cluster handling: -h, -v are no-arg; -r is required-arg.
        // For each char in cluster:
        let chars: Vec<char> = a.chars().skip(1).collect();
        let mut idx = 0;
        while idx < chars.len() {
            let c = chars[idx];
            match c {
                'h' => {
                    print_help(argv0);
                    return 0;
                }
                'v' => {
                    // verbose, no-arg, continue cluster
                    idx += 1;
                }
                'r' => {
                    // -r takes a required argument: rest of cluster, or
                    // next argv.
                    let rest = &chars[idx + 1..];
                    if !rest.is_empty() {
                        // -rfoo: rest of cluster is the value, no error.
                    } else if i + 1 < args.len() {
                        // -r foo: consume next argv.
                        i += 1;
                    } else {
                        // Missing arg → BSD's `option requires an argument`.
                        eprintln!("{argv0}: option requires an argument -- r");
                        print_help(argv0);
                        return 0;
                    }
                    break;
                }
                '-' if idx == 0 => {
                    // Long option `--foo` → unrecognized in old-style.
                    eprintln!("{argv0}: illegal option -- -");
                    print_help(argv0);
                    return 0;
                }
                other => {
                    eprintln!("{argv0}: illegal option -- {other}");
                    print_help(argv0);
                    return 0;
                }
            }
        }
        i += 1;
    }

    // Match C's behavior: pw-v4l2 is a shell wrapper that exec's the
    // remaining argv after option processing. With no command, exec
    // becomes a no-op and the wrapper exits 0 silently.
    if i >= args.len() {
        return 0;
    }
    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
}

fn print_help(argv0: &str) {
    println!("{argv0} - run v4l2 applications on PipeWire");
    println!(" ");
    println!("{argv0} [options] application [arguments]");
    println!(" ");
    println!("options:");
    println!("\t-h                  show brief help");
    println!("\t-r <remote>         remote daemon name");
    println!("\t-v                  verbose debug info");
}

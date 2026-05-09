// pw-v4l2: a setuid wrapper that LD_PRELOADs the PipeWire v4l2
// implementation before exec'ing a v4l2 client. The C tool is a tiny
// shell-style getopt wrapper; we mirror its -h output verbatim.

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-v4l2");

    for a in args.iter().skip(1) {
        match a.as_str() {
            "-h" => {
                print_help(argv0);
                return 0;
            }
            "-r" | "-v" => {}
            // pw-v4l2 uses old-style getopt (no long options), so any
            // long flag triggers the BSD-flavoured `illegal option -- -`
            // message and the help block.
            s if s.starts_with('-') => {
                let ch = if s.starts_with("--") {
                    '-'
                } else {
                    s.chars().nth(1).unwrap_or('?')
                };
                eprintln!("{argv0}: illegal option -- {ch}");
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
    println!("{argv0} - run v4l2 applications on PipeWire");
    println!(" ");
    println!("{argv0} [options] application [arguments]");
    println!(" ");
    println!("options:");
    println!("\t-h                  show brief help");
    println!("\t-r <remote>         remote daemon name");
    println!("\t-v                  verbose debug info");
}

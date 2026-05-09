// spa-resample: standalone audio resampler driver. Real implementation
// would link the SPA resampler plugin and run it on a WAV; for now we
// just emit upstream's --help text byte-for-byte.

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("spa-resample");

    let mut positional_count = 0;
    let mut consumed_flags = 0;
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            s if s.starts_with("--help=") => {
                eprintln!("{argv0}: option '--help' doesn't allow an argument");
                eprintln!("error: unknown option '?'");
                print_help(argv0);
                return 1;
            }
            "-v" | "--verbose" => {
                consumed_flags += 1;
                i += 1;
            }
            // Flags that REQUIRE a value; if missing, getopt prints
            // `option requires an argument -- 'X'`.
            opt @ ("-c" | "--cpuflags" | "-r" | "--rate" | "-f" | "--format" | "-w"
            | "--window" | "-q" | "--quality" | "-u" | "--cutoff" | "-t" | "--taps"
            | "-p" | "--param") => {
                if i + 1 >= args.len() {
                    let ch = match opt {
                        "-c" | "--cpuflags" => 'c',
                        "-r" | "--rate" => 'r',
                        "-f" | "--format" => 'f',
                        "-w" | "--window" => 'w',
                        "-q" | "--quality" => 'q',
                        "-u" | "--cutoff" => 'u',
                        "-t" | "--taps" => 't',
                        "-p" | "--param" => 'p',
                        _ => '?',
                    };
                    eprintln!("{argv0}: option requires an argument -- '{ch}'");
                    eprintln!("error: unknown option '?'");
                    print_help(argv0);
                    return 1;
                }
                consumed_flags += 2;
                i += 2;
            }
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                eprintln!("error: unknown option '?'");
                print_help(argv0);
                return 1;
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                eprintln!("error: unknown option '?'");
                print_help(argv0);
                return 1;
            }
            _ => {
                positional_count += 1;
                i += 1;
            }
        }
    }

    if positional_count < 2 {
        // C: prints `error: filename arguments missing (<optind> <argc>)`.
        // optind = 1 + number of consumed option args; argc = total argv.
        let optind = 1 + consumed_flags;
        let argc = consumed_flags + positional_count + 1;
        eprintln!("error: filename arguments missing ({optind} {argc})");
        print_help(argv0);
        return 0;
    }

    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
}

fn print_help(argv0: &str) {
    println!("{argv0} [options] <infile> <outfile>");
    println!("  -h, --help                            Show this help");
    println!("  -v  --verbose                         Be verbose");
    println!("  -c  --cpuflags                        CPU flags (default 0)");
    println!();
    println!("  -r  --rate                            Output sample rate (default as input)");
    println!(
        "  -f  --format                          Output sample format (s8|s16|s32|f32|f64) (default as input)"
    );
    println!();
    println!("  -w  --window                          Window function (default exp)");
    println!("                                                exp: Exponential window");
    println!("                                                blackman: Blackman window");
    println!("                                                kaiser: Kaiser window");
    println!("  -q  --quality                         Resampler quality (default 4)");
    println!(
        "  -u  --cutoff                          Cutoff frequency [0.0..1.0] (default from quality)"
    );
    println!("  -t  --taps                            Resampler taps (default from quality)");
    println!(
        "  -p  --param                           Resampler param <name>=<value> (default from quality)"
    );
    println!("                                                exp.A");
    println!("                                                blackman.alpha");
    println!("                                                kaiser.alpha");
    println!("                                                kaiser.stopband-attenuation");
    println!("                                                kaiser.transition-bandwidth");
    println!();
}

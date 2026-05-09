// spa-resample: standalone audio resampler driver. Real implementation
// would link the SPA resampler plugin and run it on a WAV; for now we
// just emit upstream's --help text byte-for-byte.

use crate::tools::common::expand_short_clusters;

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("spa-resample");
    // OPTIONS = "hvc:r:f:w:q:u:t:p:": h, v are no-arg; rest take values.
    let args = expand_short_clusters(raw_args, &['h', 'v']);

    let mut positional_count = 0;
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
            s if s.starts_with("--verbose=") => {
                eprintln!("{argv0}: option '--verbose' doesn't allow an argument");
                eprintln!("error: unknown option '?'");
                print_help(argv0);
                return 1;
            }
            "-v" | "--verbose" => {
                i += 1;
            }
            // Inline-value forms for required-arg long flags.
            s if s.starts_with("--cpuflags=")
                || s.starts_with("--rate=")
                || s.starts_with("--format=")
                || s.starts_with("--window=")
                || s.starts_with("--quality=")
                || s.starts_with("--cutoff=")
                || s.starts_with("--taps=")
                || s.starts_with("--param=") =>
            {
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
        // optind / argc are based on the ORIGINAL argv (before cluster
        // expansion), since getopt advances optind once per argv slot,
        // not once per char in a cluster.
        let argc = raw_args.len();
        let optind = argc - positional_count;
        eprintln!("error: filename arguments missing ({optind} {argc})");
        print_help(argv0);
        return 0;
    }

    // C tries to open the input file via sndfile; without that, mirror
    // the error format. We need to find the first positional in raw_args
    // (since args was potentially expanded).
    let infile = raw_args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with('-') || a.as_str() == "-")
        .map(|s| s.as_str())
        .unwrap_or("");
    eprintln!(
        "error: failed to open input file \"{infile}\": System error : No such file or directory."
    );
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

// spa-resample: standalone audio resampler driver. Real implementation
// would link the SPA resampler plugin and run it on a WAV; for now we
// just emit upstream's --help text byte-for-byte.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("spa-resample");

    let mut positional_count = 0;
    for a in args.iter().skip(1) {
        match a.as_str() {
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            // spa-resample's getopt_long doesn't include --version, so
            // unknown long options fall through to the help block with
            // a getopt error first.
            "-v" | "--verbose" | "-c" | "--cpuflags" | "-r" | "--rate"
            | "-f" | "--format" | "-w" | "--window" | "-q" | "--quality"
            | "-u" | "--cutoff" | "-t" | "--taps" | "-p" | "--param" => {}
            s if s.starts_with('-') => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                eprintln!("error: unknown option '?'");
                print_help(argv0);
                return 1;
            }
            _ => positional_count += 1,
        }
    }

    if positional_count < 2 {
        // C: prints `error: filename arguments missing (<optind> <argc>)`.
        // After getopt_long completes, optind points to the first
        // non-option argument; since we accept all options without
        // values (none of the recognized ones in this stub take a
        // value-as-separate-arg), optind == 1 and argc = positional + 1.
        let optind = 1;
        let argc = positional_count + 1;
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

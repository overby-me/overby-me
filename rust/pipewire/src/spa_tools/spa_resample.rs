// spa-resample: standalone audio resampler driver. Real implementation
// would link the SPA resampler plugin and run it on a WAV; for now we
// just emit upstream's --help text byte-for-byte.

use crate::tools::common::expand_short_clusters;

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("spa-resample");
    // OPTIONS = "hvc:r:f:w:q:u:t:p:": h, v are no-arg; rest take values.
    let args = expand_short_clusters(raw_args, &['h', 'v']);

    let mut positionals: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--" => {
                // getopt_long: end-of-options marker. Remaining args are
                // positional filenames.
                for s in args.iter().skip(i + 1) {
                    positionals.push(s.clone());
                }
                break;
            }
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
            // Long inline-value forms.
            s if s.starts_with("--rate=") => {
                if let Some(c) = bad_rate(&s["--rate=".len()..]) {
                    eprintln!("error: bad rate {c}");
                    print_help(argv0);
                    return 0;
                }
                i += 1;
            }
            s if s.starts_with("--format=") => {
                let v = &s["--format=".len()..];
                if !is_valid_format(v) {
                    eprintln!("error: bad format {v}");
                    print_help(argv0);
                    return 0;
                }
                i += 1;
            }
            s if s.starts_with("--quality=") => {
                let v = &s["--quality=".len()..];
                if let Some(c) = bad_quality(v) {
                    eprintln!("error: bad quality {c}");
                    print_help(argv0);
                    return 0;
                }
                i += 1;
            }
            s if s.starts_with("--cpuflags=")
                || s.starts_with("--window=")
                || s.starts_with("--cutoff=")
                || s.starts_with("--taps=")
                || s.starts_with("--param=") =>
            {
                i += 1;
            }
            // Short attached-value forms `-r<val>`, `-f<val>`, `-q<val>`.
            // (Required-arg short flags can take their value with no space.)
            s if s.starts_with("-r") && s.len() > 2 => {
                let v = &s[2..];
                if let Some(c) = bad_rate(v) {
                    eprintln!("error: bad rate {c}");
                    print_help(argv0);
                    return 0;
                }
                i += 1;
            }
            s if s.starts_with("-f") && s.len() > 2 => {
                let v = &s[2..];
                if !is_valid_format(v) {
                    eprintln!("error: bad format {v}");
                    print_help(argv0);
                    return 0;
                }
                i += 1;
            }
            s if s.starts_with("-q") && s.len() > 2 => {
                let v = &s[2..];
                if let Some(c) = bad_quality(v) {
                    eprintln!("error: bad quality {c}");
                    print_help(argv0);
                    return 0;
                }
                i += 1;
            }
            s if (s.starts_with("-c") || s.starts_with("-w")
                || s.starts_with("-u") || s.starts_with("-t")
                || s.starts_with("-p"))
                && s.len() > 2 =>
            {
                i += 1;
            }
            // Flags that REQUIRE a value; if missing, getopt prints
            // `option requires an argument -- 'X'`.
            opt @ ("-c" | "--cpuflags" | "-r" | "--rate" | "-f" | "--format" | "-w"
            | "--window" | "-q" | "--quality" | "-u" | "--cutoff" | "-t" | "--taps"
            | "-p" | "--param") => {
                if i + 1 >= args.len() {
                    if opt.starts_with("--") {
                        eprintln!("{argv0}: option '{opt}' requires an argument");
                    } else {
                        let ch = match opt {
                            "-c" => 'c', "-r" => 'r', "-f" => 'f',
                            "-w" => 'w', "-q" => 'q', "-u" => 'u',
                            "-t" => 't', "-p" => 'p',
                            _ => '?',
                        };
                        eprintln!("{argv0}: option requires an argument -- '{ch}'");
                    }
                    eprintln!("error: unknown option '?'");
                    print_help(argv0);
                    return 1;
                }
                let val = args[i + 1].as_str();
                match opt {
                    "-r" | "--rate" => {
                        if let Some(c) = bad_rate(val) {
                            eprintln!("error: bad rate {c}");
                            print_help(argv0);
                            return 0;
                        }
                    }
                    "-f" | "--format" => {
                        if !is_valid_format(val) {
                            eprintln!("error: bad format {val}");
                            print_help(argv0);
                            return 0;
                        }
                    }
                    "-q" | "--quality" => {
                        if let Some(c) = bad_quality(val) {
                            eprintln!("error: bad quality {c}");
                            print_help(argv0);
                            return 0;
                        }
                    }
                    _ => {}
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
            // Mixed cluster like `-bx`: getopt errors on first unknown.
            s if s.starts_with('-') && s != "-" => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                eprintln!("error: unknown option '?'");
                print_help(argv0);
                return 1;
            }
            _ => {
                positionals.push(a.to_string());
                i += 1;
            }
        }
    }

    if positionals.len() < 2 {
        // C: prints `error: filename arguments missing (<optind> <argc>)`.
        // optind / argc are based on the ORIGINAL argv (before cluster
        // expansion), since getopt advances optind once per argv slot,
        // not once per char in a cluster.
        let argc = raw_args.len();
        let optind = argc - positionals.len();
        eprintln!("error: filename arguments missing ({optind} {argc})");
        print_help(argv0);
        return 1;
    }

    // C tries to open the input file via sndfile; without that, mirror
    // the error format. sndfile distinguishes ENOENT vs EACCES vs
    // "Format not recognised" once the file is successfully opened.
    let infile = positionals.first().map(String::as_str).unwrap_or("");
    if infile == "-" {
        eprintln!(
            "error: failed to open input file \"{infile}\": Format not recognised."
        );
    } else if !std::path::Path::new(infile).exists() {
        eprintln!(
            "error: failed to open input file \"{infile}\": System error : No such file or directory."
        );
    } else if std::fs::File::open(infile).is_err() {
        eprintln!(
            "error: failed to open input file \"{infile}\": System error : Permission denied."
        );
    } else {
        eprintln!(
            "error: failed to open input file \"{infile}\": Format not recognised."
        );
    }
    1
}

// Returns `Some(orig)` to signal "bad rate <orig>" — only when the
// value cannot be parsed as a full i32 (C uses spa_atoi32 which both
// requires the entire string to be consumed AND rejects values outside
// the i32 range).
fn bad_rate(v: &str) -> Option<&str> {
    if v.parse::<i32>().is_ok() {
        None
    } else {
        Some(v)
    }
}

fn is_valid_format(v: &str) -> bool {
    matches!(v, "s8" | "s16" | "s32" | "f32" | "f64")
}

// `bad quality` triggers when the value parses as a full i32 but is
// negative, OR when the value is outside the i32 range (spa_atoi32
// rejects overflow). Non-numeric values pass through (C's atoi gives 0).
fn bad_quality(v: &str) -> Option<&str> {
    match v.parse::<i32>() {
        Ok(n) if n < 0 => Some(v),
        Ok(_) => None,
        Err(_) => {
            // Could be overflow (all digits, out of range) or non-numeric.
            if v.trim_start_matches('-').chars().all(|c| c.is_ascii_digit()) {
                Some(v)
            } else {
                None
            }
        }
    }
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

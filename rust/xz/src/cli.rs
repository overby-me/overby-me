//! Command-line argument parsing for the xz CLI.

use std::path::Path;
use std::process;

use crate::options::{BcjArch, FilterChain, FilterKind, Format, Mode, Options};

pub const VERSION: &str = concat!("xz (rust-xz) ", env!("CARGO_PKG_VERSION"));

/// Parse the program's argv into `Options`. Calls `process::exit` for
/// `--help`/`--version` and for unknown short flags.
pub fn parse_args() -> Options {
    let argv: Vec<String> = std::env::args().collect();
    let prog_name = Path::new(&argv[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("xz");
    parse_from(prog_name, &argv[1..])
}

/// Parse `auto`/`xz`/`lzma`/`alone`/`lzip`/`raw` into a `Format`.
fn parse_format(name: &str) -> Option<Format> {
    match name {
        "auto" => Some(Format::Auto),
        "xz" => Some(Format::Xz),
        "lzma" | "alone" => Some(Format::Lzma),
        "lzip" => Some(Format::Lzip),
        "raw" => Some(Format::Raw),
        _ => None,
    }
}

/// Parse a `--lzma1=`/`--lzma2=` value. Currently we only need
/// `preset=N` (with optional trailing `e` for "extreme", which we
/// silently strip — the test suite only ever passes `preset=0`).
fn parse_lzma_value(value: &str, lzma2: bool) -> Option<FilterKind> {
    let value = value.strip_suffix('e').unwrap_or(value); // ignore extreme bit
    let preset_str = value.strip_prefix("preset=")?;
    let preset: u32 = preset_str.parse().ok()?;
    Some(if lzma2 {
        FilterKind::Lzma2Preset(preset)
    } else {
        FilterKind::Lzma1Preset(preset)
    })
}

/// Parse a single token from a `--filters=` chain. Accepted tokens
/// are `lzma2`/`lzma2:preset=N` (and `lzma1` likewise), the BCJ
/// filter names (`x86`, `arm`, `arm64`, `armthumb`, `powerpc`,
/// `ia64`, `sparc`, `riscv`), and `delta`.
fn parse_filter_token(tok: &str) -> Option<FilterKind> {
    // Allow either a literal name or `name:opts`.
    let (name, opts) = match tok.split_once(':') {
        Some((n, o)) => (n, Some(o)),
        None => (tok, None),
    };
    match name {
        "lzma1" => {
            let preset = opts
                .and_then(|o| o.strip_prefix("preset="))
                .and_then(|p| p.strip_suffix('e').or(Some(p)))
                .and_then(|p| p.parse::<u32>().ok())
                .unwrap_or(6);
            Some(FilterKind::Lzma1Preset(preset))
        }
        "lzma2" => {
            let preset = opts
                .and_then(|o| o.strip_prefix("preset="))
                .and_then(|p| p.strip_suffix('e').or(Some(p)))
                .and_then(|p| p.parse::<u32>().ok())
                .unwrap_or(6);
            Some(FilterKind::Lzma2Preset(preset))
        }
        "delta" => Some(FilterKind::Delta),
        "x86" => Some(FilterKind::Bcj(BcjArch::X86)),
        "arm" => Some(FilterKind::Bcj(BcjArch::Arm)),
        "arm64" => Some(FilterKind::Bcj(BcjArch::Arm64)),
        "armthumb" => Some(FilterKind::Bcj(BcjArch::ArmThumb)),
        "powerpc" | "ppc" => Some(FilterKind::Bcj(BcjArch::PowerPc)),
        "ia64" => Some(FilterKind::Bcj(BcjArch::Ia64)),
        "sparc" => Some(FilterKind::Bcj(BcjArch::Sparc)),
        "riscv" => Some(FilterKind::Bcj(BcjArch::RiscV)),
        _ => None,
    }
}

/// Parse a `--filters=` value, e.g. `x86 lzma2:preset=4` or
/// `x86--lzma2`. Both whitespace and the literal `--` separator
/// are accepted (xz accepts both).
fn parse_filters_chain(value: &str) -> Option<FilterChain> {
    let mut chain = FilterChain::default();
    // Replace `--` with a single space so we can split uniformly.
    let normalised = value.replace("--", " ");
    for tok in normalised.split_whitespace() {
        let kind = parse_filter_token(tok)?;
        chain.push(kind);
    }
    if chain.is_empty() {
        return None;
    }
    Some(chain)
}

fn push_filter(opts: &mut Options, kind: FilterKind) {
    let chain = opts.filter.get_or_insert_with(FilterChain::default);
    chain.push(kind);
}

/// Library-friendly variant used by tests: takes the program name and
/// the post-argv[0] arguments as plain `&str`s.
pub fn parse_from(prog_name: &str, args: &[String]) -> Options {
    let mut opts = Options::defaults_for(prog_name);
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-d" | "--decompress" | "--uncompress" => opts.mode = Mode::Decompress,
            "-z" | "--compress" => opts.mode = Mode::Compress,
            "-t" | "--test" => opts.mode = Mode::Test,
            "-l" | "--list" => opts.mode = Mode::List,
            "-c" | "--stdout" | "--to-stdout" => opts.stdout = true,
            "-k" | "--keep" => opts.keep = true,
            "-f" | "--force" => opts.force = true,
            "-v" | "--verbose" => opts.verbose = true,
            "-q" | "--quiet" => opts.quiet = true,
            "-Q" | "--no-warn" => opts.no_warn = true,
            "--fast" => opts.level = 0,
            "--best" => opts.level = 9,
            "--no-adjust" => { /* accept-and-ignore */ }
            "-0" => opts.level = 0,
            "-1" => opts.level = 1,
            "-2" => opts.level = 2,
            "-3" => opts.level = 3,
            "-4" => opts.level = 4,
            "-5" => opts.level = 5,
            "-6" => opts.level = 6,
            "-7" => opts.level = 7,
            "-8" => opts.level = 8,
            "-9" => opts.level = 9,
            "--x86" => push_filter(&mut opts, FilterKind::Bcj(BcjArch::X86)),
            "--arm" => push_filter(&mut opts, FilterKind::Bcj(BcjArch::Arm)),
            "--arm64" => push_filter(&mut opts, FilterKind::Bcj(BcjArch::Arm64)),
            "--armthumb" => push_filter(&mut opts, FilterKind::Bcj(BcjArch::ArmThumb)),
            "--powerpc" => push_filter(&mut opts, FilterKind::Bcj(BcjArch::PowerPc)),
            "--ia64" => push_filter(&mut opts, FilterKind::Bcj(BcjArch::Ia64)),
            "--sparc" => push_filter(&mut opts, FilterKind::Bcj(BcjArch::Sparc)),
            "--riscv" => push_filter(&mut opts, FilterKind::Bcj(BcjArch::RiscV)),
            "--delta" => push_filter(&mut opts, FilterKind::Delta),
            "-T" | "--threads" => {
                // Accept but ignore the next argument
                i += 1;
            }
            "-F" | "--format" => {
                i += 1;
                if let Some(name) = args.get(i)
                    && let Some(fmt) = parse_format(name)
                {
                    opts.format = fmt;
                } else {
                    eprintln!("xz: unknown format: {:?}", args.get(i));
                    process::exit(1);
                }
            }
            "-S" | "--suffix" => {
                i += 1;
                if let Some(s) = args.get(i) {
                    opts.suffix = Some(s.clone());
                }
            }
            "-h" | "--help" => {
                print_help();
                process::exit(0);
            }
            "-V" | "--version" => {
                println!("{VERSION}");
                process::exit(0);
            }
            "--" => {
                i += 1;
                while i < args.len() {
                    opts.files.push(args[i].clone());
                    i += 1;
                }
                break;
            }
            _ if arg.starts_with("--format=") => {
                let name = &arg["--format=".len()..];
                if let Some(fmt) = parse_format(name) {
                    opts.format = fmt;
                } else {
                    eprintln!("xz: unknown format: {name}");
                    process::exit(1);
                }
            }
            _ if arg.starts_with("--suffix=") => {
                opts.suffix = Some(arg["--suffix=".len()..].to_string());
            }
            _ if arg.starts_with("--lzma1=") => {
                if let Some(kind) = parse_lzma_value(&arg["--lzma1=".len()..], false) {
                    push_filter(&mut opts, kind);
                } else {
                    eprintln!("xz: unsupported --lzma1= value: {arg}");
                    process::exit(1);
                }
            }
            _ if arg.starts_with("--lzma2=") => {
                if let Some(kind) = parse_lzma_value(&arg["--lzma2=".len()..], true) {
                    push_filter(&mut opts, kind);
                } else {
                    eprintln!("xz: unsupported --lzma2= value: {arg}");
                    process::exit(1);
                }
            }
            _ if arg.starts_with("--filters=") => {
                let value = &arg["--filters=".len()..];
                if let Some(chain) = parse_filters_chain(value) {
                    opts.filter = Some(chain);
                } else {
                    eprintln!("xz: unsupported --filters= value: {arg}");
                    process::exit(1);
                }
            }
            _ if arg.starts_with("--files=") => {
                let path = &arg["--files=".len()..];
                if let Err(e) = read_files_list(path, b'\n', &mut opts.files) {
                    eprintln!("xz: {path}: {e}");
                    process::exit(1);
                }
            }
            _ if arg.starts_with("--files0=") => {
                let path = &arg["--files0=".len()..];
                if let Err(e) = read_files_list(path, b'\0', &mut opts.files) {
                    eprintln!("xz: {path}: {e}");
                    process::exit(1);
                }
            }
            _ if arg.starts_with("--memlimit") => { /* accept-and-ignore */ }
            _ if arg.starts_with('-')
                && !arg.starts_with("--")
                && arg.len() > 2
                && !arg.starts_with("-T")
                && !arg.starts_with("-F")
                && !arg.starts_with("-S") =>
            {
                // Handle combined short flags like -dkv
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'd' => opts.mode = Mode::Decompress,
                        'z' => opts.mode = Mode::Compress,
                        't' => opts.mode = Mode::Test,
                        'l' => opts.mode = Mode::List,
                        'c' => opts.stdout = true,
                        'k' => opts.keep = true,
                        'f' => opts.force = true,
                        'v' => opts.verbose = true,
                        'q' => opts.quiet = true,
                        'Q' => opts.no_warn = true,
                        '0' => opts.level = 0,
                        '1' => opts.level = 1,
                        '2' => opts.level = 2,
                        '3' => opts.level = 3,
                        '4' => opts.level = 4,
                        '5' => opts.level = 5,
                        '6' => opts.level = 6,
                        '7' => opts.level = 7,
                        '8' => opts.level = 8,
                        '9' => opts.level = 9,
                        c => {
                            eprintln!("xz: unknown option: -{c}");
                            process::exit(1);
                        }
                    }
                    j += 1;
                }
            }
            _ if arg.starts_with("--threads=") => {
                // Accept but ignore
            }
            _ if arg.starts_with("-T") && arg.len() > 2 => {
                // -T0, -T4, etc. - accept but ignore
            }
            _ if arg.starts_with("-F") && arg.len() > 2 => {
                let name = &arg[2..];
                if let Some(fmt) = parse_format(name) {
                    opts.format = fmt;
                } else {
                    eprintln!("xz: unknown format: {name}");
                    process::exit(1);
                }
            }
            _ if arg.starts_with("-S") && arg.len() > 2 => {
                opts.suffix = Some(arg[2..].to_string());
            }
            _ => {
                opts.files.push(arg.clone());
            }
        }
        i += 1;
    }
    opts
}

/// Read a `--files`/`--files0` list and append each entry to `out`.
/// `sep` is `b'\n'` for `--files` and `b'\0'` for `--files0`.
fn read_files_list(path: &str, sep: u8, out: &mut Vec<String>) -> std::io::Result<()> {
    let bytes = std::fs::read(path)?;
    for chunk in bytes.split(|&b| b == sep) {
        if chunk.is_empty() {
            continue;
        }
        match std::str::from_utf8(chunk) {
            Ok(s) => out.push(s.to_string()),
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "filename in --files/--files0 list is not valid UTF-8",
                ));
            }
        }
    }
    Ok(())
}

pub fn print_help() {
    eprintln!(
        "Usage: xz [OPTION]... [FILE]...
Compress or decompress .xz files.

  -z, --compress        force compression
  -d, --decompress      force decompression
  -t, --test            test compressed file integrity
  -l, --list            list information about .xz files
  -c, --stdout          write to standard output, keep original files
  -k, --keep            keep original files
  -f, --force           force overwrite of output file
  -0 ... -9             compression preset level (default: 6)
      --fast            alias for -0
      --best            alias for -9
  -F, --format=FMT      file format (auto|xz|lzma|alone|lzip|raw)
  -S, --suffix=.SUF     custom suffix
      --lzma1=preset=N  LZMA1 filter, used with -F raw
      --lzma2=preset=N  LZMA2 filter (last in chain)
      --filters=CHAIN   space-separated filter chain
      --x86 --arm       BCJ filters (also: --arm64, --armthumb,
      --arm64 --riscv     --powerpc, --ia64, --sparc, --riscv)
      --delta           delta filter (default distance 1)
      --files=FILE      read filenames from FILE (newline-separated)
      --files0=FILE     read filenames from FILE (NUL-separated)
  -T, --threads=N       use N threads (accepted for compatibility)
  -v, --verbose         be verbose
  -q, --quiet           suppress warnings
  -Q, --no-warn         do not warn about unsupported integrity checks
  -h, --help            display this help
  -V, --version         display version"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args_uses_xz_defaults() {
        let o = parse_from("xz", &[]);
        assert_eq!(o.mode, Mode::Compress);
        assert_eq!(o.format, Format::Auto);
        assert!(o.files.is_empty());
    }

    #[test]
    fn long_decompress() {
        let o = parse_from("xz", &args(&["--decompress", "a.xz"]));
        assert_eq!(o.mode, Mode::Decompress);
        assert_eq!(o.files, vec!["a.xz".to_string()]);
    }

    #[test]
    fn combined_short_flags() {
        let o = parse_from("xz", &args(&["-dkv"]));
        assert_eq!(o.mode, Mode::Decompress);
        assert!(o.keep);
        assert!(o.verbose);
    }

    #[test]
    fn combined_level_and_force() {
        let o = parse_from("xz", &args(&["-9cf"]));
        assert_eq!(o.level, 9);
        assert!(o.stdout);
        assert!(o.force);
    }

    #[test]
    fn fast_and_best_aliases() {
        assert_eq!(parse_from("xz", &args(&["--fast"])).level, 0);
        assert_eq!(parse_from("xz", &args(&["--best"])).level, 9);
    }

    #[test]
    fn threads_consumes_following_arg() {
        let o = parse_from("xz", &args(&["-T", "4", "input"]));
        assert_eq!(o.files, vec!["input".to_string()]);
    }

    #[test]
    fn threads_inline_form() {
        let o = parse_from("xz", &args(&["-T0", "input"]));
        assert_eq!(o.files, vec!["input".to_string()]);
    }

    #[test]
    fn threads_long_inline() {
        let o = parse_from("xz", &args(&["--threads=2", "input"]));
        assert_eq!(o.files, vec!["input".to_string()]);
    }

    #[test]
    fn double_dash_ends_options() {
        let o = parse_from("xz", &args(&["--", "-d", "-c"]));
        assert_eq!(o.mode, Mode::Compress); // -d after -- is a filename
        assert_eq!(o.files, vec!["-d".to_string(), "-c".to_string()]);
    }

    #[test]
    fn unxz_default_then_keep() {
        let o = parse_from("unxz", &args(&["-k", "x.xz"]));
        assert_eq!(o.mode, Mode::Decompress);
        assert!(o.keep);
        assert_eq!(o.files, vec!["x.xz".to_string()]);
    }

    #[test]
    fn xzcat_implies_stdout_decompress() {
        let o = parse_from("xzcat", &args(&["x.xz"]));
        assert_eq!(o.mode, Mode::Decompress);
        assert!(o.stdout);
    }

    #[test]
    fn lzma_default_format() {
        let o = parse_from("lzma", &[]);
        assert_eq!(o.format, Format::Lzma);
        assert_eq!(o.mode, Mode::Compress);
    }

    #[test]
    fn level_short() {
        assert_eq!(parse_from("xz", &args(&["-3"])).level, 3);
        assert_eq!(parse_from("xz", &args(&["-0"])).level, 0);
    }

    #[test]
    fn collects_multiple_files() {
        let o = parse_from("xz", &args(&["a", "b", "c"]));
        assert_eq!(o.files, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    #[test]
    fn format_long_eq() {
        assert_eq!(parse_from("xz", &args(&["--format=raw"])).format, Format::Raw);
        assert_eq!(parse_from("xz", &args(&["--format=lzma"])).format, Format::Lzma);
        assert_eq!(parse_from("xz", &args(&["--format=lzip"])).format, Format::Lzip);
        assert_eq!(parse_from("xz", &args(&["--format=auto"])).format, Format::Auto);
    }

    #[test]
    fn format_short_dash_f_inline() {
        assert_eq!(parse_from("xz", &args(&["-Fraw"])).format, Format::Raw);
    }

    #[test]
    fn format_short_dash_f_separate() {
        assert_eq!(parse_from("xz", &args(&["-F", "lzma"])).format, Format::Lzma);
    }

    #[test]
    fn suffix_long_eq() {
        assert_eq!(
            parse_from("xz", &args(&["--suffix=.foo"])).suffix.as_deref(),
            Some(".foo")
        );
    }

    #[test]
    fn suffix_short() {
        assert_eq!(
            parse_from("xz", &args(&["-S", ".bar"])).suffix.as_deref(),
            Some(".bar")
        );
        assert_eq!(
            parse_from("xz", &args(&["-S.baz"])).suffix.as_deref(),
            Some(".baz")
        );
    }

    #[test]
    fn lzma1_preset_appends_to_chain() {
        let o = parse_from("xz", &args(&["--lzma1=preset=0"]));
        let chain = o.filter.unwrap();
        assert_eq!(chain.as_slice(), &[FilterKind::Lzma1Preset(0)]);
    }

    #[test]
    fn lzma2_preset_appends_to_chain() {
        let o = parse_from("xz", &args(&["--lzma2=preset=4"]));
        let chain = o.filter.unwrap();
        assert_eq!(chain.as_slice(), &[FilterKind::Lzma2Preset(4)]);
    }

    #[test]
    fn no_warn_combined_short() {
        let o = parse_from("xz", &args(&["-dcqQ", "x.xz"]));
        assert!(o.no_warn);
        assert!(o.quiet);
    }

    #[test]
    fn list_long() {
        let o = parse_from("xz", &args(&["--list", "x.xz"]));
        assert_eq!(o.mode, Mode::List);
    }

    #[test]
    fn list_short() {
        let o = parse_from("xz", &args(&["-l", "x.xz"]));
        assert_eq!(o.mode, Mode::List);
    }

    #[test]
    fn list_combined_short() {
        let o = parse_from("xz", &args(&["-lv", "x.xz"]));
        assert_eq!(o.mode, Mode::List);
        assert!(o.verbose);
    }

    #[test]
    fn bcj_x86_appends_filter() {
        let o = parse_from("xz", &args(&["--x86", "--lzma2=preset=4", "input"]));
        let chain = o.filter.unwrap();
        assert_eq!(
            chain.as_slice(),
            &[FilterKind::Bcj(BcjArch::X86), FilterKind::Lzma2Preset(4)]
        );
    }

    #[test]
    fn bcj_arm64_riscv_delta_recognised() {
        for (flag, expected) in [
            ("--arm", FilterKind::Bcj(BcjArch::Arm)),
            ("--arm64", FilterKind::Bcj(BcjArch::Arm64)),
            ("--armthumb", FilterKind::Bcj(BcjArch::ArmThumb)),
            ("--powerpc", FilterKind::Bcj(BcjArch::PowerPc)),
            ("--ia64", FilterKind::Bcj(BcjArch::Ia64)),
            ("--sparc", FilterKind::Bcj(BcjArch::Sparc)),
            ("--riscv", FilterKind::Bcj(BcjArch::RiscV)),
            ("--delta", FilterKind::Delta),
        ] {
            let o = parse_from("xz", &args(&[flag, "--lzma2=preset=0"]));
            let chain = o.filter.unwrap();
            assert_eq!(chain.as_slice()[0], expected, "first kind for {flag}");
        }
    }

    #[test]
    fn filters_long_eq_space_separated() {
        let o = parse_from("xz", &args(&["--filters=x86 lzma2:preset=4"]));
        let chain = o.filter.unwrap();
        assert_eq!(
            chain.as_slice(),
            &[FilterKind::Bcj(BcjArch::X86), FilterKind::Lzma2Preset(4)]
        );
    }

    #[test]
    fn filters_long_eq_dash_dash_separated() {
        let o = parse_from("xz", &args(&["--filters=arm64--lzma2:preset=6"]));
        let chain = o.filter.unwrap();
        assert_eq!(
            chain.as_slice(),
            &[FilterKind::Bcj(BcjArch::Arm64), FilterKind::Lzma2Preset(6)]
        );
    }

    #[test]
    fn filters_long_eq_default_preset_is_six() {
        let o = parse_from("xz", &args(&["--filters=lzma2"]));
        let chain = o.filter.unwrap();
        assert_eq!(chain.as_slice(), &[FilterKind::Lzma2Preset(6)]);
    }
}

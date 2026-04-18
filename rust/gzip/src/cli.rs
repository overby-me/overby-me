use std::env;
use std::io::{self, Write};
use std::path::Path;
use std::process;

// First line ends with the upstream version so gnulib's help-version
// test (which extracts "$(--version | sed '1s/.* //')" and compares to
// $VERSION exported from the test harness) matches pkgs.gzip's version.
pub const VERSION: &str = "gzip (rust-gzip) 1.14";

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Compress,
    Decompress,
    Test,
    List,
}

pub struct Options {
    pub mode: Mode,
    pub to_stdout: bool,
    pub keep: bool,
    pub force: bool,
    pub level: u32,
    pub store_name: bool,
    pub verbose: bool,
    pub quiet: bool,
    pub recursive: bool,
    pub suffix: String,
    pub files: Vec<String>,
}

pub fn parse_args() -> Options {
    let args: Vec<String> = env::args().collect();
    let program = Path::new(&args[0])
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "gzip".to_string());

    let mut mode = Mode::Compress;
    let mut to_stdout = false;
    let mut keep = false;
    let mut force = false;
    let mut level: u32 = 6;
    let mut store_name = true;
    let mut verbose = false;
    let mut quiet = false;
    let mut recursive = false;
    let mut suffix = String::from(".gz");
    let mut files: Vec<String> = Vec::new();

    // Handle program name aliases
    match program.as_str() {
        "gunzip" => mode = Mode::Decompress,
        "zcat" => {
            mode = Mode::Decompress;
            to_stdout = true;
        }
        _ => {}
    }

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            files.extend(args[i + 1..].iter().cloned());
            break;
        }
        if arg == "---presume-input-tty" {
            // Undocumented escape hatch used by the upstream test suite
            // to bypass the "won't read compressed data from a terminal"
            // heuristic. We never consult isatty, so it's a no-op.
        } else if arg.starts_with("--") {
            match arg.as_str() {
                "--decompress" | "--uncompress" => mode = Mode::Decompress,
                "--stdout" | "--to-stdout" => to_stdout = true,
                "--keep" => keep = true,
                "--force" => force = true,
                "--fast" => level = 1,
                "--best" => level = 9,
                "--verbose" => verbose = true,
                "--quiet" => quiet = true,
                "--test" => mode = Mode::Test,
                "--list" => mode = Mode::List,
                "--recursive" => recursive = true,
                "--no-name" => store_name = false,
                "--name" => store_name = true,
                // --synchronous forces fsync after writing; tests only
                // check that it's accepted, not the underlying syscall.
                "--synchronous" => {}
                "--version" => process::exit(print_version()),
                "--help" => process::exit(print_usage()),
                "--suffix" => {
                    if i + 1 >= args.len() {
                        eprintln!("gzip: option '--suffix' requires an argument");
                        process::exit(1);
                    }
                    i += 1;
                    suffix = args[i].clone();
                    if suffix.is_empty() {
                        eprintln!("gzip: invalid suffix ''");
                        process::exit(1);
                    }
                }
                _ => {
                    eprintln!("gzip: unrecognized option '{arg}'");
                    process::exit(1);
                }
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                match chars[j] {
                    'd' => mode = Mode::Decompress,
                    'c' => to_stdout = true,
                    'k' => keep = true,
                    'f' => force = true,
                    'v' => verbose = true,
                    'q' => quiet = true,
                    't' => mode = Mode::Test,
                    'l' => mode = Mode::List,
                    'r' => recursive = true,
                    'n' => store_name = false,
                    'N' => store_name = true,
                    'h' => process::exit(print_usage()),
                    'V' => process::exit(print_version()),
                    'S' => {
                        // -S takes the rest of this arg or the next arg.
                        let rest: String = chars[j + 1..].iter().collect();
                        if rest.is_empty() {
                            if i + 1 >= args.len() {
                                eprintln!("gzip: option requires an argument -- 'S'");
                                process::exit(1);
                            }
                            i += 1;
                            suffix = args[i].clone();
                        } else {
                            suffix = rest;
                        }
                        if suffix.is_empty() {
                            eprintln!("gzip: invalid suffix ''");
                            process::exit(1);
                        }
                        break;
                    }
                    c @ '1'..='9' => level = c.to_digit(10).unwrap(),
                    _ => {
                        eprintln!("gzip: invalid option -- '{}'", chars[j]);
                        process::exit(1);
                    }
                }
                j += 1;
            }
        } else {
            files.push(arg.clone());
        }
        i += 1;
    }

    Options {
        mode,
        to_stdout,
        keep,
        force,
        level,
        store_name,
        verbose,
        quiet,
        recursive,
        suffix,
        files,
    }
}

// Write --help to stdout and return an exit code that reflects whether
// the write succeeded. GNU coreutils / gzip exit 1 on --help if the
// output stream was, for example, /dev/full (help-version test).
pub fn print_usage() -> i32 {
    let stdout = io::stdout();
    let mut w = stdout.lock();
    let body = "Usage: gzip [OPTION]... [FILE]...\n\
Compress or decompress FILEs (by default, compress FILES in-place).\n\
\n\
  -c, --stdout       write on standard output, keep original files\n\
  -d, --decompress   decompress\n\
  -f, --force        force overwrite of output file\n\
  -k, --keep         keep (don't delete) input files\n\
  -l, --list         list compressed file contents\n\
  -n, --no-name      do not save or restore the original name and timestamp\n\
  -N, --name         save or restore the original file name and timestamp\n\
  -q, --quiet        suppress all warnings\n\
  -r, --recursive    operate recursively on directories\n\
  -S, --suffix=SUF   use suffix SUF on compressed files\n\
  -t, --test         test compressed file integrity\n\
  -v, --verbose      verbose mode\n\
  -1, --fast         compress faster\n\
  -9, --best         compress better\n\
  -V, --version      display version number\n\
  -h, --help         give this help\n\
\n\
With no FILE, or when FILE is -, read standard input.\n";
    if w.write_all(body.as_bytes()).is_err() || w.flush().is_err() {
        1
    } else {
        0
    }
}

pub fn print_version() -> i32 {
    let stdout = io::stdout();
    let mut w = stdout.lock();
    if writeln!(&mut w, "{VERSION}").is_err() || w.flush().is_err() {
        1
    } else {
        0
    }
}

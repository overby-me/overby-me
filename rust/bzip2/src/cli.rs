use std::env;
use std::path::Path;
use std::process;

pub struct Config {
    pub decompress: bool,
    pub stdout: bool,
    pub keep: bool,
    pub force: bool,
    pub level: u32,
    pub verbose: bool,
    pub test: bool,
    pub quiet: bool,
    pub small: bool,
    pub files: Vec<String>,
}

pub fn parse_args() -> Config {
    let args: Vec<String> = env::args().collect();
    let prog_name = Path::new(&args[0])
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut config = Config {
        decompress: false,
        stdout: false,
        keep: false,
        force: false,
        level: 9,
        verbose: false,
        test: false,
        quiet: false,
        small: false,
        files: Vec::new(),
    };

    // Set defaults based on program name
    if prog_name.contains("bunzip2") {
        config.decompress = true;
    } else if prog_name.contains("bzcat") {
        config.decompress = true;
        config.stdout = true;
    }

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-d" | "--decompress" | "--uncompress" => config.decompress = true,
            "-z" | "--compress" => config.decompress = false,
            "-c" | "--stdout" | "--to-stdout" => config.stdout = true,
            "-k" | "--keep" => config.keep = true,
            "-f" | "--force" => config.force = true,
            "-v" | "--verbose" => config.verbose = true,
            "-t" | "--test" => {
                config.test = true;
                config.decompress = true;
            }
            "-q" | "--quiet" => config.quiet = true,
            "-s" | "--small" => config.small = true,
            "-1" | "--fast" => config.level = 1,
            "-2" => config.level = 2,
            "-3" => config.level = 3,
            "-4" => config.level = 4,
            "-5" => config.level = 5,
            "-6" => config.level = 6,
            "-7" => config.level = 7,
            "-8" => config.level = 8,
            "-9" | "--best" => config.level = 9,
            "--version" | "-V" => {
                eprintln!("bzip2 (rust-bzip2) {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "-h" | "--help" => {
                print_usage();
                process::exit(0);
            }
            "--" => {
                config.files.extend_from_slice(&args[i + 1..]);
                break;
            }
            _ if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2 => {
                // Handle combined short flags like -dkv
                for ch in arg[1..].chars() {
                    match ch {
                        'd' => config.decompress = true,
                        'z' => config.decompress = false,
                        'c' => config.stdout = true,
                        'k' => config.keep = true,
                        'f' => config.force = true,
                        'v' => config.verbose = true,
                        't' => {
                            config.test = true;
                            config.decompress = true;
                        }
                        'q' => config.quiet = true,
                        's' => config.small = true,
                        '1' => config.level = 1,
                        '2' => config.level = 2,
                        '3' => config.level = 3,
                        '4' => config.level = 4,
                        '5' => config.level = 5,
                        '6' => config.level = 6,
                        '7' => config.level = 7,
                        '8' => config.level = 8,
                        '9' => config.level = 9,
                        _ => {
                            eprintln!("bzip2: unknown flag '{ch}'");
                            process::exit(1);
                        }
                    }
                }
            }
            _ if arg.starts_with('-') => {
                eprintln!("bzip2: unknown option '{arg}'");
                process::exit(1);
            }
            _ => config.files.push(arg.clone()),
        }
        i += 1;
    }

    config
}

pub fn print_usage() {
    eprintln!(
        "Usage: bzip2 [OPTIONS] [FILE]...
Compress or decompress bzip2 files.

Options:
  -d, --decompress   Decompress
  -z, --compress     Compress (default)
  -c, --stdout       Write to stdout
  -k, --keep         Keep input files
  -f, --force        Force overwrite
  -t, --test         Test integrity
  -v, --verbose      Verbose output
  -q, --quiet        Quiet mode
  -s, --small        Use less memory
  -1 .. -9           Compression level (default: -9)
      --fast          Alias for -1
      --best          Alias for -9
  -V, --version      Show version
  -h, --help         Show this help"
    );
}

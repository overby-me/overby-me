use bzip2::Compression;
use bzip2::read::{BzDecoder, BzEncoder};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

struct Config {
    decompress: bool,
    stdout: bool,
    keep: bool,
    force: bool,
    level: u32,
    verbose: bool,
    test: bool,
    quiet: bool,
    small: bool,
    files: Vec<String>,
}

fn parse_args() -> Config {
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

fn print_usage() {
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

fn compress(input: &mut dyn Read, output: &mut dyn Write, level: u32) -> io::Result<()> {
    let mut encoder = BzEncoder::new(input, Compression::new(level));
    io::copy(&mut encoder, output)?;
    Ok(())
}

fn decompress(input: &mut dyn Read, output: &mut dyn Write) -> io::Result<()> {
    let mut decoder = BzDecoder::new(input);
    io::copy(&mut decoder, output)?;
    Ok(())
}

fn process_file(config: &Config, path: &str) -> io::Result<()> {
    let input_path = PathBuf::from(path);

    if !input_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("bzip2: {path}: No such file or directory"),
        ));
    }

    if !input_path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bzip2: {path}: Not a regular file"),
        ));
    }

    if config.decompress {
        let ext = input_path.extension().unwrap_or_default();
        if ext != "bz2" && ext != "bz" && ext != "tbz2" && ext != "tbz" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("bzip2: {path}: unknown suffix -- ignored"),
            ));
        }

        let output_path = match ext.to_str().unwrap_or("") {
            "bz2" => {
                let s = input_path.to_string_lossy();
                PathBuf::from(s.strip_suffix(".bz2").unwrap())
            }
            "bz" => {
                let s = input_path.to_string_lossy();
                PathBuf::from(s.strip_suffix(".bz").unwrap())
            }
            "tbz2" => input_path.with_extension("tar"),
            "tbz" => input_path.with_extension("tar"),
            _ => unreachable!(),
        };

        if config.test {
            let mut input = File::open(&input_path)?;
            decompress(&mut input, &mut io::sink())?;
            if config.verbose {
                eprintln!("{path}: ok");
            }
        } else if config.stdout {
            let mut input = File::open(&input_path)?;
            let mut stdout = io::stdout().lock();
            decompress(&mut input, &mut stdout)?;
        } else {
            if output_path.exists() && !config.force {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "bzip2: output file {} already exists",
                        output_path.display()
                    ),
                ));
            }
            let mut input = File::open(&input_path)?;
            let mut output = File::create(&output_path)?;
            decompress(&mut input, &mut output)?;
            if !config.keep {
                fs::remove_file(&input_path)?;
            }
            if config.verbose {
                eprintln!("  {path}: done", path = path);
            }
        }
    } else {
        let output_path = PathBuf::from(format!("{path}.bz2"));

        if config.stdout {
            let mut input = File::open(&input_path)?;
            let mut stdout = io::stdout().lock();
            compress(&mut input, &mut stdout, config.level)?;
        } else {
            if output_path.exists() && !config.force {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "bzip2: output file {} already exists",
                        output_path.display()
                    ),
                ));
            }

            // Preserve permissions
            let metadata = fs::metadata(&input_path)?;
            let mut input = File::open(&input_path)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&output_path)?;
            compress(&mut input, &mut output, config.level)?;

            // Copy permissions
            fs::set_permissions(&output_path, metadata.permissions())?;

            if !config.keep {
                fs::remove_file(&input_path)?;
            }
            if config.verbose {
                eprintln!("  {path}: done");
            }
        }
    }

    Ok(())
}

fn main() {
    let config = parse_args();

    if config.files.is_empty() {
        // Read from stdin, write to stdout
        let mut stdin = io::stdin().lock();
        let mut stdout = io::stdout().lock();
        let result = if config.decompress {
            if config.test {
                decompress(&mut stdin, &mut io::sink())
            } else {
                decompress(&mut stdin, &mut stdout)
            }
        } else {
            compress(&mut stdin, &mut stdout, config.level)
        };

        if let Err(e) = result {
            if !config.quiet {
                eprintln!("bzip2: {e}");
            }
            process::exit(1);
        }
    } else {
        let mut had_error = false;
        for file in &config.files {
            if let Err(e) = process_file(&config, file) {
                if !config.quiet {
                    eprintln!("{e}");
                }
                had_error = true;
            }
        }
        if had_error {
            process::exit(1);
        }
    }
}

use std::env;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use xz2::read::{XzDecoder, XzEncoder};

const VERSION: &str = "rust-xz 0.1.0";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Compress,
    Decompress,
    Test,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Format {
    Xz,
    Lzma,
}

struct Options {
    mode: Mode,
    stdout: bool,
    keep: bool,
    force: bool,
    level: u32,
    verbose: bool,
    quiet: bool,
    format: Format,
    files: Vec<String>,
}

fn parse_args() -> Options {
    let args: Vec<String> = env::args().collect();

    // Determine defaults from argv[0]
    let prog_name = Path::new(&args[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("xz");

    let (default_mode, default_stdout, default_format) = match prog_name {
        "unxz" => (Mode::Decompress, false, Format::Xz),
        "xzcat" => (Mode::Decompress, true, Format::Xz),
        "lzma" => (Mode::Compress, false, Format::Lzma),
        "unlzma" => (Mode::Decompress, false, Format::Lzma),
        "lzcat" => (Mode::Decompress, true, Format::Lzma),
        _ => (Mode::Compress, false, Format::Xz),
    };

    let mut opts = Options {
        mode: default_mode,
        stdout: default_stdout,
        keep: false,
        force: false,
        level: 6,
        verbose: false,
        quiet: false,
        format: default_format,
        files: Vec::new(),
    };

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-d" | "--decompress" | "--uncompress" => opts.mode = Mode::Decompress,
            "-z" | "--compress" => opts.mode = Mode::Compress,
            "-t" | "--test" => opts.mode = Mode::Test,
            "-c" | "--stdout" | "--to-stdout" => opts.stdout = true,
            "-k" | "--keep" => opts.keep = true,
            "-f" | "--force" => opts.force = true,
            "-v" | "--verbose" => opts.verbose = true,
            "-q" | "--quiet" => opts.quiet = true,
            "--fast" => opts.level = 0,
            "--best" => opts.level = 9,
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
            "-T" | "--threads" => {
                // Accept but ignore the next argument
                i += 1;
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
            _ if arg.starts_with("-") && !arg.starts_with("--") && arg.len() > 2 => {
                // Handle combined short flags like -dkv
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'd' => opts.mode = Mode::Decompress,
                        'z' => opts.mode = Mode::Compress,
                        't' => opts.mode = Mode::Test,
                        'c' => opts.stdout = true,
                        'k' => opts.keep = true,
                        'f' => opts.force = true,
                        'v' => opts.verbose = true,
                        'q' => opts.quiet = true,
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
                        'T' => {
                            // If T is the last char in the combined flags, consume next arg
                            if j + 1 == chars.len() {
                                i += 1;
                            }
                            // Otherwise just skip (e.g. -T0 is threads=0)
                            break;
                        }
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
            _ => {
                opts.files.push(arg.clone());
            }
        }
        i += 1;
    }

    opts
}

fn print_help() {
    eprintln!(
        "Usage: xz [OPTION]... [FILE]...
Compress or decompress .xz files.

  -z, --compress      force compression
  -d, --decompress    force decompression
  -t, --test          test compressed file integrity
  -c, --stdout        write to standard output, keep original files
  -k, --keep          keep original files
  -f, --force         force overwrite of output file
  -0 ... -9           compression preset level (default: 6)
      --fast           alias for -0
      --best           alias for -9
  -T, --threads=N     use N threads (accepted for compatibility)
  -v, --verbose       be verbose
  -q, --quiet         suppress warnings
  -h, --help          display this help
  -V, --version       display version"
    );
}

fn compress_stream<R: Read, W: Write>(input: R, output: W, level: u32) -> io::Result<()> {
    let encoder = XzEncoder::new(input, level);
    let mut reader = BufReader::new(encoder);
    let mut writer = BufWriter::new(output);
    io::copy(&mut reader, &mut writer)?;
    writer.flush()?;
    Ok(())
}

fn decompress_stream<R: Read, W: Write>(input: R, output: W) -> io::Result<()> {
    let decoder = XzDecoder::new(input);
    let mut reader = BufReader::new(decoder);
    let mut writer = BufWriter::new(output);
    io::copy(&mut reader, &mut writer)?;
    writer.flush()?;
    Ok(())
}

fn suffix_for_format(format: Format) -> &'static str {
    match format {
        Format::Xz => ".xz",
        Format::Lzma => ".lzma",
    }
}

fn compressed_suffixes() -> &'static [&'static str] {
    &[".xz", ".lzma", ".txz", ".tlz"]
}

fn output_path_compress(input: &Path, format: Format) -> PathBuf {
    let suffix = suffix_for_format(format);
    let mut out = input.as_os_str().to_owned();
    out.push(suffix);
    PathBuf::from(out)
}

fn output_path_decompress(input: &Path) -> Option<PathBuf> {
    let name = input.to_str()?;
    if let Some(stripped) = name.strip_suffix(".xz") {
        Some(PathBuf::from(stripped))
    } else if let Some(stripped) = name.strip_suffix(".lzma") {
        Some(PathBuf::from(stripped))
    } else {
        name.strip_suffix(".txz")
            .or_else(|| name.strip_suffix(".tlz"))
            .map(|stripped| PathBuf::from(format!("{stripped}.tar")))
    }
}

fn process_file(path: &str, opts: &Options) -> io::Result<()> {
    let input_path = Path::new(path);

    if !input_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{path}: No such file or directory"),
        ));
    }

    match opts.mode {
        Mode::Compress => {
            let has_suffix = compressed_suffixes().iter().any(|s| path.ends_with(s));
            if has_suffix && !opts.force {
                if !opts.quiet {
                    eprintln!("xz: {path}: already has a compressed suffix, skipping");
                }
                return Ok(());
            }

            let output_path = output_path_compress(input_path, opts.format);

            if opts.stdout {
                let input = File::open(input_path)?;
                compress_stream(input, io::stdout().lock(), opts.level)?;
            } else {
                if output_path.exists() && !opts.force {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "{}: already exists; use -f to overwrite",
                            output_path.display()
                        ),
                    ));
                }
                let input = File::open(input_path)?;
                let output = File::create(&output_path)?;
                compress_stream(input, output, opts.level)?;

                if opts.verbose {
                    eprintln!("xz: {path} -> {}", output_path.display());
                }

                if !opts.keep {
                    fs::remove_file(input_path)?;
                }
            }
        }
        Mode::Decompress => {
            let output_path = match output_path_decompress(input_path) {
                Some(p) => p,
                None => {
                    if !opts.quiet {
                        eprintln!("xz: {path}: unknown suffix -- ignored");
                    }
                    return Ok(());
                }
            };

            if opts.stdout {
                let input = File::open(input_path)?;
                decompress_stream(input, io::stdout().lock())?;
            } else {
                if output_path.exists() && !opts.force {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "{}: already exists; use -f to overwrite",
                            output_path.display()
                        ),
                    ));
                }
                let input = File::open(input_path)?;
                let output = File::create(&output_path)?;
                decompress_stream(input, output)?;

                if opts.verbose {
                    eprintln!("xz: {} -> {}", path, output_path.display());
                }

                if !opts.keep {
                    fs::remove_file(input_path)?;
                }
            }
        }
        Mode::Test => {
            let input = File::open(input_path)?;
            decompress_stream(input, io::sink())?;
            if opts.verbose {
                eprintln!("xz: {path}: OK");
            }
        }
    }

    Ok(())
}

fn main() {
    let opts = parse_args();

    if opts.files.is_empty() {
        // Read from stdin, write to stdout
        let stdin = io::stdin().lock();
        let stdout = io::stdout().lock();

        let result = match opts.mode {
            Mode::Compress => compress_stream(stdin, stdout, opts.level),
            Mode::Decompress | Mode::Test => {
                if opts.mode == Mode::Test {
                    decompress_stream(stdin, io::sink())
                } else {
                    decompress_stream(stdin, stdout)
                }
            }
        };

        if let Err(e) = result {
            if !opts.quiet {
                eprintln!("xz: {e}");
            }
            process::exit(1);
        }
    } else {
        let mut errors = false;
        for file in &opts.files {
            if file == "-" {
                let stdin = io::stdin().lock();
                let stdout = io::stdout().lock();
                let result = match opts.mode {
                    Mode::Compress => compress_stream(stdin, stdout, opts.level),
                    Mode::Decompress => decompress_stream(stdin, stdout),
                    Mode::Test => decompress_stream(stdin, io::sink()),
                };
                if let Err(e) = result {
                    if !opts.quiet {
                        eprintln!("xz: stdin: {e}");
                    }
                    errors = true;
                }
            } else if let Err(e) = process_file(file, &opts) {
                if !opts.quiet {
                    eprintln!("xz: {e}");
                }
                errors = true;
            }
        }
        if errors {
            process::exit(1);
        }
    }
}

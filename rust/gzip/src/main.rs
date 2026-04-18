use flate2::read::{GzDecoder, MultiGzDecoder};
use flate2::{Compression, GzBuilder};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::SystemTime;

const VERSION: &str = "gzip (rust-gzip) 0.1.0";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Compress,
    Decompress,
    Test,
    List,
}

struct Options {
    mode: Mode,
    to_stdout: bool,
    keep: bool,
    force: bool,
    level: u32,
    store_name: bool,
    verbose: bool,
    quiet: bool,
    recursive: bool,
    files: Vec<String>,
}

fn parse_args() -> Options {
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
        if arg.starts_with("--") {
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
                "--version" => {
                    println!("{VERSION}");
                    process::exit(0);
                }
                "--help" => {
                    print_usage();
                    process::exit(0);
                }
                "--suffix" => {
                    i += 1; // consume suffix arg, we always use .gz
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
                    'h' => {
                        print_usage();
                        process::exit(0);
                    }
                    'V' => {
                        println!("{VERSION}");
                        process::exit(0);
                    }
                    'S' => {
                        // consume next arg as suffix, we always use .gz
                        i += 1;
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
        files,
    }
}

fn print_usage() {
    eprintln!(
        "Usage: gzip [OPTION]... [FILE]...
Compress or decompress FILEs (by default, compress FILES in-place).

  -c, --stdout       write on standard output, keep original files
  -d, --decompress   decompress
  -f, --force        force overwrite of output file
  -k, --keep         keep (don't delete) input files
  -l, --list         list compressed file contents
  -n, --no-name      do not save or restore the original name and timestamp
  -N, --name         save or restore the original file name and timestamp
  -q, --quiet        suppress all warnings
  -r, --recursive    operate recursively on directories
  -t, --test         test compressed file integrity
  -v, --verbose      verbose mode
  -1, --fast         compress faster
  -9, --best         compress better
  -V, --version      display version number
  -h, --help         give this help

With no FILE, or when FILE is -, read standard input."
    );
}

fn main() {
    let opts = parse_args();

    let exit_code = if opts.files.is_empty() || (opts.files.len() == 1 && opts.files[0] == "-") {
        run_stdio(&opts)
    } else {
        run_files(&opts)
    };

    process::exit(exit_code);
}

fn run_stdio(opts: &Options) -> i32 {
    let stdin = io::stdin();
    let stdout = io::stdout();

    match opts.mode {
        Mode::Compress => {
            let reader = stdin.lock();
            let writer = stdout.lock();
            if let Err(e) = compress_stream(reader, writer, opts.level, None, opts.store_name) {
                eprintln!("gzip: {e}");
                return 1;
            }
        }
        Mode::Decompress => {
            let reader = stdin.lock();
            let writer = stdout.lock();
            if let Err(e) = decompress_stream(reader, writer) {
                eprintln!("gzip: {e}");
                return 1;
            }
        }
        Mode::Test => {
            let reader = stdin.lock();
            if let Err(e) = decompress_stream(reader, io::sink()) {
                eprintln!("gzip: stdin: {e}");
                return 1;
            }
        }
        Mode::List => {
            eprintln!("gzip: stdin: not in gzip format or cannot list from stdin");
            return 1;
        }
    }
    0
}

fn run_files(opts: &Options) -> i32 {
    let mut exit_code = 0;

    for path_str in &opts.files {
        if path_str == "-" {
            if run_stdio(opts) != 0 {
                exit_code = 1;
            }
            continue;
        }

        let path = Path::new(path_str);

        if path.is_dir() {
            if opts.recursive {
                if process_dir(path, opts) != 0 {
                    exit_code = 1;
                }
            } else {
                eprintln!("gzip: {path_str}: is a directory -- ignored");
                exit_code = 1;
            }
            continue;
        }

        if process_file(path, opts) != 0 {
            exit_code = 1;
        }
    }

    exit_code
}

fn process_dir(dir: &Path, opts: &Options) -> i32 {
    let mut exit_code = 0;
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("gzip: {}: {e}", dir.display());
            return 1;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("gzip: {e}");
                exit_code = 1;
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            if process_dir(&path, opts) != 0 {
                exit_code = 1;
            }
        } else if process_file(&path, opts) != 0 {
            exit_code = 1;
        }
    }

    exit_code
}

fn process_file(path: &Path, opts: &Options) -> i32 {
    match opts.mode {
        Mode::Compress => compress_file(path, opts),
        Mode::Decompress => decompress_file(path, opts),
        Mode::Test => test_file(path, opts),
        Mode::List => list_file(path, opts),
    }
}

fn compress_file(path: &Path, opts: &Options) -> i32 {
    let out_path = PathBuf::from(format!("{}.gz", path.display()));

    if !opts.to_stdout && !opts.force && out_path.exists() {
        eprintln!(
            "gzip: {} already exists; not overwriting",
            out_path.display()
        );
        return 1;
    }

    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    if !metadata.is_file() {
        eprintln!("gzip: {}: not a regular file -- ignored", path.display());
        return 1;
    }

    let input = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    let original_size = metadata.len();
    let file_name = path.file_name().map(|s| s.to_string_lossy().to_string());

    if opts.to_stdout {
        let stdout = io::stdout();
        if let Err(e) = compress_stream(
            BufReader::new(input),
            stdout.lock(),
            opts.level,
            file_name.as_deref(),
            opts.store_name,
        ) {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    } else {
        let output = match create_output_file(&out_path, opts.force) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("gzip: {}: {e}", out_path.display());
                return 1;
            }
        };

        if let Err(e) = compress_stream(
            BufReader::new(input),
            BufWriter::new(output),
            opts.level,
            file_name.as_deref(),
            opts.store_name,
        ) {
            eprintln!("gzip: {}: {e}", path.display());
            let _ = fs::remove_file(&out_path);
            return 1;
        }

        // Copy permissions
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&out_path, meta.permissions());
        }

        if !opts.keep
            && let Err(e) = fs::remove_file(path)
        {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }

        if opts.verbose {
            let compressed_size = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            let ratio = if original_size == 0 {
                0.0
            } else {
                (1.0 - compressed_size as f64 / original_size as f64) * 100.0
            };
            eprintln!(
                "{}: {ratio:.1}% -- replaced with {}",
                path.display(),
                out_path.display()
            );
        }
    }

    0
}

fn decompress_file(path: &Path, opts: &Options) -> i32 {
    let path_str = path.to_string_lossy();

    // Only compute the output path when we'll actually write a file.
    // With -c/--stdout we decompress regardless of suffix.
    let out_path = if opts.to_stdout {
        PathBuf::new()
    } else if let Some(stem) = strip_gz_suffix(path) {
        PathBuf::from(stem)
    } else {
        if !opts.quiet {
            eprintln!("gzip: {path_str}: unknown suffix -- ignored");
        }
        return 1;
    };

    if !opts.to_stdout && !opts.force && out_path.exists() {
        eprintln!(
            "gzip: {} already exists; not overwriting",
            out_path.display()
        );
        return 1;
    }

    let input = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gzip: {path_str}: {e}");
            return 1;
        }
    };

    if opts.to_stdout {
        let stdout = io::stdout();
        if let Err(e) = decompress_stream(BufReader::new(input), stdout.lock()) {
            eprintln!("gzip: {path_str}: {e}");
            return 1;
        }
    } else {
        let output = match create_output_file(&out_path, opts.force) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("gzip: {}: {e}", out_path.display());
                return 1;
            }
        };

        if let Err(e) = decompress_stream(BufReader::new(input), BufWriter::new(output)) {
            eprintln!("gzip: {path_str}: {e}");
            let _ = fs::remove_file(&out_path);
            return 1;
        }

        // Copy permissions
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&out_path, meta.permissions());
        }

        if !opts.keep
            && let Err(e) = fs::remove_file(path)
        {
            eprintln!("gzip: {path_str}: {e}");
            return 1;
        }

        if opts.verbose {
            eprintln!("{path_str}: -- replaced with {}", out_path.display());
        }
    }

    0
}

fn test_file(path: &Path, opts: &Options) -> i32 {
    let input = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    if let Err(e) = decompress_stream(BufReader::new(input), io::sink()) {
        eprintln!("gzip: {}: {e}", path.display());
        return 1;
    }

    if opts.verbose {
        eprintln!("{}: OK", path.display());
    }

    0
}

fn list_file(path: &Path, opts: &Options) -> i32 {
    let input = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    let compressed_size = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    // Read the header to get original name
    let mut decoder = GzDecoder::new(BufReader::new(input));
    let mut buf = Vec::new();
    if let Err(e) = decoder.read_to_end(&mut buf) {
        eprintln!("gzip: {}: {e}", path.display());
        return 1;
    }

    let uncompressed_size = buf.len() as u64;
    let ratio = if uncompressed_size == 0 {
        0.0
    } else {
        (1.0 - compressed_size as f64 / uncompressed_size as f64) * 100.0
    };

    let out_name = strip_gz_suffix(path).unwrap_or_else(|| path.to_string_lossy().to_string());

    if !opts.quiet {
        println!("  compressed  uncompressed  ratio  uncompressed_name");
    }
    println!("  {compressed_size:>10}  {uncompressed_size:>12}  {ratio:>5.1}%  {out_name}");

    0
}

fn compress_stream<R: Read, W: Write>(
    mut reader: R,
    writer: W,
    level: u32,
    file_name: Option<&str>,
    store_name: bool,
) -> io::Result<()> {
    let mut builder = GzBuilder::new();
    if store_name {
        if let Some(name) = file_name {
            builder = builder.filename(name);
        }
        let mtime = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        builder = builder.mtime(mtime);
    }

    let mut encoder = builder.write(writer, Compression::new(level));
    io::copy(&mut reader, &mut encoder)?;
    encoder.finish()?;
    Ok(())
}

fn decompress_stream<R: Read, W: Write>(reader: R, mut writer: W) -> io::Result<()> {
    let mut decoder = MultiGzDecoder::new(reader);
    io::copy(&mut decoder, &mut writer)?;
    writer.flush()?;
    Ok(())
}

fn strip_gz_suffix(path: &Path) -> Option<String> {
    let s = path.to_string_lossy();
    for suffix in &[".gz", ".tgz", ".z", ".Z", "-gz", "-z", "_z"] {
        if let Some(stem) = s.strip_suffix(suffix) {
            let result = if *suffix == ".tgz" {
                format!("{stem}.tar")
            } else {
                stem.to_string()
            };
            return Some(result);
        }
    }
    None
}

fn create_output_file(path: &Path, force: bool) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true);
    if force {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    opts.open(path)
}
